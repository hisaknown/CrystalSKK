//! ネットの候補が届くのを待つ時計 (ADR-0042)。
//!
//! 辞書サーバは外の答えを待たずに「取りに行っている」と返す。届いたかは
//! 尋ね直さないと分からないので、待っているあいだだけ短い間隔で尋ねる。
//! 届いたら、待たせた打鍵の続きをして見せる ([`crystalskk_core::Engine::poll_web`])。
//!
//! 時計は窓を持たないスレッドの時計 (`SetTimer` に窓を渡さない) にする。
//! **打鍵を受けたのと同じスレッドで鳴る**ので、TSF の呼び出しと重ならない。

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::TextServices::ITfContext;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::ComObject;

use crate::guard::guard;
use crate::log;
use crate::service::TextService;

/// 尋ねる間隔。
const INTERVAL_MS: u32 = 100;

/// 待つ上限。過ぎたら尋ねるのをやめる。もう一度送れば、また尋ねる。
const LIMIT: Duration = Duration::from_secs(10);

/// 待っているもの。
struct Waiting {
    service: ComObject<TextService>,
    context: ITfContext,
    since: Instant,
    timer: usize,
}

thread_local! {
    static WAITING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
}

/// 待ち始める。もう待っていれば、入力先と上限を改める。
pub fn start(service: ComObject<TextService>, context: ITfContext) {
    stop();
    // SAFETY: 窓を持たない時計。止めるのは `stop` で、同じスレッドから。
    let timer = unsafe { SetTimer(None, 0, INTERVAL_MS, Some(tick)) };
    if timer == 0 {
        log::error("ネットの候補を待つ時計を掛けられなかった");
        return;
    }
    WAITING.with_borrow_mut(|waiting| {
        *waiting = Some(Waiting {
            service,
            context,
            since: Instant::now(),
            timer,
        });
    });
}

/// 待つのをやめる。待っていなければ何もしない。
pub fn stop() {
    if let Some(waiting) = WAITING.with_borrow_mut(Option::take) {
        // SAFETY: 自分で掛けた時計を外すだけ。
        let _ = unsafe { KillTimer(None, waiting.timer) };
    }
}

unsafe extern "system" fn tick(_hwnd: HWND, _message: u32, _id: usize, _time: u32) {
    // 借りたまま呼ぶと、呼んだ先で `stop` されたときに借用が重なる。
    let Some((service, context, since)) = WAITING.with_borrow(|waiting| {
        waiting
            .as_ref()
            .map(|w| (w.service.clone(), w.context.clone(), w.since))
    }) else {
        return;
    };
    if since.elapsed() > LIMIT {
        log::trace("ネットの候補を待ちきれなかった");
        stop();
        return;
    }
    let _ = guard("webpoll::tick", || {
        service.poll_web(&context);
        Ok(())
    });
}
