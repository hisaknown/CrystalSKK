//! 取得先 URL の分解。
//!
//! 辞書の取得にしか使わないので、必要な範囲だけを扱う。認証情報付きの
//! URL や、http/https 以外の scheme は受け付けない。

use crate::Error;

/// 分解した URL。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// パスとクエリ。先頭の `/` を含む。
    pub target: String,
    pub secure: bool,
}

impl Url {
    pub fn parse(url: &str) -> Result<Self, Error> {
        let (scheme, rest) = url.split_once("://").ok_or_else(|| Error::bad_url(url))?;
        let secure = match scheme.to_ascii_lowercase().as_str() {
            "https" => true,
            "http" => false,
            _ => return Err(Error::bad_url(url)),
        };

        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        if authority.contains('@') {
            return Err(Error::bad_url(url));
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port.parse::<u16>().map_err(|_| Error::bad_url(url))?;
                (host, port)
            }
            None => (authority, if secure { 443 } else { 80 }),
        };
        if host.is_empty() {
            return Err(Error::bad_url(url));
        }

        Ok(Self {
            host: host.to_owned(),
            port,
            target: path.to_owned(),
            secure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_https_url() {
        let url = Url::parse("https://raw.githubusercontent.com/skk-dev/dict/master/SKK-JISYO.L")
            .expect("読める");
        assert_eq!(url.host, "raw.githubusercontent.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.target, "/skk-dev/dict/master/SKK-JISYO.L");
        assert!(url.secure);
    }

    #[test]
    fn defaults_the_path_and_port() {
        let url = Url::parse("http://example.com").expect("読める");
        assert_eq!(url.target, "/");
        assert_eq!(url.port, 80);
        assert!(!url.secure);
    }

    #[test]
    fn accepts_an_explicit_port() {
        let url = Url::parse("https://example.com:8443/dict").expect("読める");
        assert_eq!(url.port, 8443);
        assert_eq!(url.target, "/dict");
    }

    #[test]
    fn keeps_the_query_string() {
        let url = Url::parse("https://example.com/d?ref=main").expect("読める");
        assert_eq!(url.target, "/d?ref=main");
    }

    #[test]
    fn rejects_what_it_cannot_handle() {
        assert!(Url::parse("ftp://example.com/d").is_err());
        assert!(Url::parse("example.com/d").is_err());
        assert!(Url::parse("https:///d").is_err());
        // 認証情報付きの URL は扱わない。
        assert!(Url::parse("https://user:pass@example.com/d").is_err());
    }
}
