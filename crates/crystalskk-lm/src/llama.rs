//! llama.cpp のビルド済み DLL を呼ぶ。
//!
//! **C++ はビルドしない** (ADR-0030)。公式のビルド済み `llama.dll` を実行時に
//! 読み、使う関数だけを手で宣言する。構造体の形は llama.cpp の版ごとに
//! 変わりうるので、[`VERSION`] の版に合わせてある。版を上げるときは
//! `include/llama.h` と見比べて、ここを合わせ直すこと。
//!
//! unsafe はこのモジュールに閉じる。外に見せるのは [`Model::score`] だけ。

#![allow(unsafe_code)]

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::time::Instant;

use libloading::Library;

/// 構造体の形を合わせた llama.cpp の版。
pub const VERSION: &str = "b11124";

/// 一度の decode に載せられるトークンの数。
pub const MAX_BATCH: usize = 512;

/// 同時に持てる列の数。0 番が共有する前の文章、1 番から先が候補。
pub const MAX_SEQUENCES: usize = 16;

/// KV キャッシュの大きさ。前の文章と、全候補の残りが収まればよい。
const CONTEXT_SIZE: u32 = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
struct ModelParams {
    devices: *mut c_void,
    tensor_buft_overrides: *const c_void,
    n_gpu_layers: i32,
    split_mode: i32,
    load_mode: i32,
    lazy_mode: i32,
    main_gpu: i32,
    tensor_split: *const f32,
    progress_callback: *mut c_void,
    progress_callback_user_data: *mut c_void,
    kv_overrides: *const c_void,
    vocab_only: bool,
    check_tensors: bool,
    use_extra_bufts: bool,
    no_host: bool,
    no_alloc: bool,
    load_mtp: bool,
}

type AbortCallback = unsafe extern "C" fn(*mut c_void) -> bool;

#[repr(C)]
#[derive(Clone, Copy)]
struct ContextParams {
    n_ctx: u32,
    n_batch: u32,
    n_ubatch: u32,
    n_seq_max: u32,
    n_rs_seq: u32,
    n_outputs_max: u32,
    n_outputs_max_per_seq: u32,
    n_threads: i32,
    n_threads_batch: i32,
    ctx_type: i32,
    rope_scaling_type: i32,
    pooling_type: i32,
    attention_type: i32,
    flash_attn_type: i32,
    rope_freq_base: f32,
    rope_freq_scale: f32,
    yarn_ext_factor: f32,
    yarn_attn_factor: f32,
    yarn_beta_fast: f32,
    yarn_beta_slow: f32,
    yarn_orig_ctx: u32,
    defrag_thold: f32,
    cb_eval: *mut c_void,
    cb_eval_user_data: *mut c_void,
    type_k: i32,
    type_v: i32,
    abort_callback: Option<AbortCallback>,
    abort_callback_data: *mut c_void,
    embeddings: bool,
    offload_kqv: bool,
    no_perf: bool,
    op_offload: bool,
    swa_full: bool,
    kv_unified: bool,
    samplers: *mut c_void,
    n_samplers: usize,
    ctx_other: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Batch {
    n_tokens: i32,
    token: *mut i32,
    embd: *mut f32,
    pos: *mut i32,
    n_seq_id: *mut i32,
    seq_id: *mut *mut i32,
    logits: *mut i8,
}

type LogCallback = unsafe extern "C" fn(i32, *const c_char, *mut c_void);

/// 使う関数。DLL が読まれているあいだだけ有効なので、[`Runtime`] が
/// DLL と一緒に持つ。
struct Functions {
    model_default_params: unsafe extern "C" fn() -> ModelParams,
    context_default_params: unsafe extern "C" fn() -> ContextParams,
    model_load_from_file: unsafe extern "C" fn(*const c_char, ModelParams) -> *mut c_void,
    model_free: unsafe extern "C" fn(*mut c_void),
    init_from_model: unsafe extern "C" fn(*mut c_void, ContextParams) -> *mut c_void,
    free: unsafe extern "C" fn(*mut c_void),
    model_get_vocab: unsafe extern "C" fn(*const c_void) -> *const c_void,
    vocab_n_tokens: unsafe extern "C" fn(*const c_void) -> i32,
    get_memory: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    memory_clear: unsafe extern "C" fn(*mut c_void, bool),
    memory_seq_cp: unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32),
    batch_init: unsafe extern "C" fn(i32, i32, i32) -> Batch,
    batch_free: unsafe extern "C" fn(Batch),
    decode: unsafe extern "C" fn(*mut c_void, Batch) -> i32,
    get_logits_ith: unsafe extern "C" fn(*mut c_void, i32) -> *mut f32,
}

/// 読み込んだ llama.cpp。
struct Runtime {
    f: Functions,
    // 関数より後に落とす。
    _llama: Library,
    _ggml: Library,
}

/// 何も書かない記録係。サーバの標準エラーを llama.cpp の記録で埋めない。
unsafe extern "C" fn quiet(_level: i32, _text: *const c_char, _data: *mut c_void) {}

/// 締め切りを過ぎたら計算をやめさせる。`data` は [`Model::deadline`]。
unsafe extern "C" fn past_deadline(data: *mut c_void) -> bool {
    // SAFETY: `data` は Model が箱に入れて持ち続ける Cell を指す。
    let deadline = unsafe { &*(data as *const Cell<Option<Instant>>) };
    deadline.get().is_some_and(|at| Instant::now() >= at)
}

/// DLL を読む。依存する DLL (ggml-base など) は同じフォルダから探させる。
fn open(path: &Path) -> Result<Library, String> {
    #[cfg(windows)]
    let library = {
        use libloading::os::windows::{LOAD_WITH_ALTERED_SEARCH_PATH, Library as WinLibrary};
        // SAFETY: 読むのは利用者が設定で指した llama.cpp の DLL である。
        unsafe { WinLibrary::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH) }
            .map(Library::from)
    };
    #[cfg(not(windows))]
    // SAFETY: 同上。
    let library = unsafe { Library::new(path) };
    library.map_err(|e| format!("{} を読めません: {e}", path.display()))
}

impl Runtime {
    fn load(directory: &Path) -> Result<Self, String> {
        let ggml = open(&directory.join("ggml.dll"))?;
        let llama = open(&directory.join("llama.dll"))?;
        let missing =
            |name: &str, e: libloading::Error| format!("llama.dll に {name} がありません ({e})");

        macro_rules! get {
            ($lib:expr, $name:literal) => {
                // SAFETY: 宣言した型は VERSION の llama.h と合わせてある。
                *unsafe { $lib.get($name.as_bytes()) }.map_err(|e| missing($name, e))?
            };
        }

        // CPU の実装は別の DLL に分かれていて、CPU に合うものを選んで読む。
        let load_all: unsafe extern "C" fn(*const c_char) =
            get!(ggml, "ggml_backend_load_all_from_path");
        let directory_c = CString::new(directory.to_string_lossy().as_bytes())
            .map_err(|_| "llama.cpp のフォルダの名前に NUL が入っています".to_owned())?;
        let backend_init: unsafe extern "C" fn() = get!(llama, "llama_backend_init");
        let log_set: unsafe extern "C" fn(Option<LogCallback>, *mut c_void) =
            get!(llama, "llama_log_set");
        // SAFETY: どれも引数の決まった初期化で、何度呼んでもよい。
        unsafe {
            log_set(Some(quiet), std::ptr::null_mut());
            load_all(directory_c.as_ptr());
            backend_init();
        }

        let f = Functions {
            model_default_params: get!(llama, "llama_model_default_params"),
            context_default_params: get!(llama, "llama_context_default_params"),
            model_load_from_file: get!(llama, "llama_model_load_from_file"),
            model_free: get!(llama, "llama_model_free"),
            init_from_model: get!(llama, "llama_init_from_model"),
            free: get!(llama, "llama_free"),
            model_get_vocab: get!(llama, "llama_model_get_vocab"),
            vocab_n_tokens: get!(llama, "llama_vocab_n_tokens"),
            get_memory: get!(llama, "llama_get_memory"),
            memory_clear: get!(llama, "llama_memory_clear"),
            memory_seq_cp: get!(llama, "llama_memory_seq_cp"),
            batch_init: get!(llama, "llama_batch_init"),
            batch_free: get!(llama, "llama_batch_free"),
            decode: get!(llama, "llama_decode"),
            get_logits_ith: get!(llama, "llama_get_logits_ith"),
        };
        Ok(Self {
            f,
            _llama: llama,
            _ggml: ggml,
        })
    }
}

/// 読み込んだ言語モデル。
pub struct Model {
    runtime: Runtime,
    model: *mut c_void,
    context: *mut c_void,
    memory: *mut c_void,
    batch: Batch,
    n_vocab: usize,
    /// 締め切り。abort のコールバックが読むので、動かない場所に置く。
    deadline: Box<Cell<Option<Instant>>>,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("n_vocab", &self.n_vocab)
            .finish_non_exhaustive()
    }
}

impl Model {
    /// `runtime` のフォルダの llama.cpp で、`gguf` を読む。CPU だけで動かす。
    pub fn load(runtime: &Path, gguf: &Path, threads: usize) -> Result<Self, String> {
        let runtime = Runtime::load(runtime)?;
        let f = &runtime.f;
        let path = CString::new(gguf.to_string_lossy().as_bytes())
            .map_err(|_| "言語モデルの名前に NUL が入っています".to_owned())?;
        let deadline = Box::new(Cell::new(None));
        let threads = i32::try_from(threads).unwrap_or(i32::MAX);

        // SAFETY: 既定値を受け取り、決まった欄だけを書き換えて渡す。返った
        // ポインタは Drop で対になる関数に返す。
        unsafe {
            let mut mp = (f.model_default_params)();
            mp.n_gpu_layers = 0;
            let model = (f.model_load_from_file)(path.as_ptr(), mp);
            if model.is_null() {
                return Err(format!("{} を言語モデルとして読めません", gguf.display()));
            }
            let mut cp = (f.context_default_params)();
            cp.n_ctx = CONTEXT_SIZE;
            cp.n_batch = MAX_BATCH as u32;
            cp.n_ubatch = MAX_BATCH as u32;
            cp.n_seq_max = MAX_SEQUENCES as u32;
            cp.n_threads = threads;
            cp.n_threads_batch = threads;
            // 全候補が長い前の文章を共有するので、一つの領域にまとめる。
            cp.kv_unified = true;
            cp.no_perf = true;
            cp.abort_callback = Some(past_deadline);
            cp.abort_callback_data = &*deadline as *const Cell<Option<Instant>> as *mut c_void;
            let context = (f.init_from_model)(model, cp);
            if context.is_null() {
                (f.model_free)(model);
                return Err("言語モデルの計算の場を用意できません".to_owned());
            }
            let n_vocab = (f.vocab_n_tokens)((f.model_get_vocab)(model));
            let memory = (f.get_memory)(context);
            let batch = (f.batch_init)(MAX_BATCH as i32, 0, MAX_SEQUENCES as i32);
            Ok(Self {
                n_vocab: usize::try_from(n_vocab).unwrap_or(0),
                model,
                context,
                memory,
                batch,
                deadline,
                runtime,
            })
        }
    }

    /// 前の文章 `prefix` に続けて、それぞれの `suffixes` が来る確率の対数を返す。
    ///
    /// `prefix` は全候補で共有するトークン、`suffixes` は候補ごとの残り
    /// (空であってはならない)。締め切りまでに終わらなければ `None`。
    pub fn score(
        &mut self,
        prefix: &[i32],
        suffixes: &[Vec<i32>],
        deadline: Instant,
    ) -> Option<Vec<f32>> {
        let total = prefix.len() + suffixes.iter().map(Vec::len).sum::<usize>();
        if prefix.is_empty()
            || suffixes.is_empty()
            || suffixes.len() >= MAX_SEQUENCES
            || suffixes.iter().any(Vec::is_empty)
            || prefix.len() > MAX_BATCH
            || total > CONTEXT_SIZE as usize
        {
            return None;
        }
        self.deadline.set(Some(deadline));
        let (memory_clear, memory_seq_cp) =
            (self.runtime.f.memory_clear, self.runtime.f.memory_seq_cp);
        // SAFETY: memory は context のもので、context より先には落ちない。
        unsafe { memory_clear(self.memory, true) };

        let last = prefix.len() - 1;
        let entries: Vec<_> = prefix
            .iter()
            .enumerate()
            .map(|(p, &t)| (t, p as i32, 0, p == last))
            .collect();
        if !self.decode(&entries) {
            return None;
        }
        // 前の文章の最後のトークンが、各候補の最初のトークンを予測する。
        let first: Vec<f32> = suffixes
            .iter()
            .map(|s| self.log_prob(last as i32, s[0]))
            .collect();

        for seq in 1..=suffixes.len() {
            // SAFETY: seq は MAX_SEQUENCES 未満。
            unsafe { memory_seq_cp(self.memory, 0, seq as i32, -1, -1) };
        }
        let mut entries = Vec::new();
        let mut starts = Vec::new();
        for (k, s) in suffixes.iter().enumerate() {
            starts.push(entries.len());
            for (j, &t) in s.iter().enumerate() {
                // 候補の最後のトークンの出力は使わない。出力層 (語彙の数だけ
                // ある) は重いので省く。
                entries.push((t, (prefix.len() + j) as i32, k as i32 + 1, j + 1 < s.len()));
            }
        }
        if entries.len() > MAX_BATCH || !self.decode(&entries) {
            return None;
        }
        self.deadline.set(None);

        Some(
            suffixes
                .iter()
                .enumerate()
                .map(|(k, s)| {
                    first[k]
                        + (1..s.len())
                            .map(|j| self.log_prob((starts[k] + j - 1) as i32, s[j]))
                            .sum::<f32>()
                })
                .collect(),
        )
    }

    /// `(トークン, 位置, 列, 出力が要るか)` を一度に流す。
    fn decode(&mut self, entries: &[(i32, i32, i32, bool)]) -> bool {
        let b = self.batch;
        // SAFETY: batch は MAX_BATCH トークン、一つにつき列 MAX_SEQUENCES
        // 個ぶんで確保してあり、entries はそれを越えない (呼ぶ側で確かめる)。
        unsafe {
            for (i, &(token, pos, seq, output)) in entries.iter().enumerate() {
                *b.token.add(i) = token;
                *b.pos.add(i) = pos;
                *b.n_seq_id.add(i) = 1;
                *(*b.seq_id.add(i)) = seq;
                *b.logits.add(i) = i8::from(output);
            }
            let mut batch = b;
            batch.n_tokens = entries.len() as i32;
            (self.runtime.f.decode)(self.context, batch) == 0
        }
    }

    /// 直前の decode で `i` 番目のトークンが出した分布での、`token` の対数確率。
    fn log_prob(&self, i: i32, token: i32) -> f32 {
        // SAFETY: i は出力を頼んだトークンの番号で、分布は語彙の数だけある。
        let row = unsafe {
            let ptr = (self.runtime.f.get_logits_ith)(self.context, i);
            if ptr.is_null() {
                return f32::NEG_INFINITY;
            }
            std::slice::from_raw_parts(ptr, self.n_vocab)
        };
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = row.iter().map(|x| (x - max).exp()).sum();
        match usize::try_from(token).ok().and_then(|t| row.get(t)) {
            Some(x) => x - max - sum.ln(),
            None => f32::NEG_INFINITY,
        }
    }
}

// SAFETY: 生のポインタを持つので自動では `Send` にならない。llama.cpp の
// モデルと文脈は作ったスレッドに縛られない (スレッドごとの領域を使わない)
// ので、**同時に触らない限り**別のスレッドへ渡してよい。`Sync` にはしない
// ので、同時に触ることは型が許さない。サーバは裏のスレッドで読み込み、
// 答えるスレッドへ渡す (ADR-0030)。
unsafe impl Send for Model {}

impl Drop for Model {
    fn drop(&mut self) {
        let f = &self.runtime.f;
        // SAFETY: 確保した順の逆に、対になる関数へ返す。
        unsafe {
            (f.batch_free)(self.batch);
            (f.free)(self.context);
            (f.model_free)(self.model);
        }
    }
}
