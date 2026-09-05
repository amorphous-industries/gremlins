use std::fmt;

const BUILTIN_SCHEMES: &[&str] = &["file", "git", "gh"];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Uri {
    pub scheme: String,
    pub path: String,
}

impl Uri {
    pub fn new(scheme: String, path: String) -> Self {
        Uri { scheme, path }
    }

    pub fn parse(s: &str) -> Result<Self, UriError> {
        let (scheme, path) = s
            .split_once("://")
            .ok_or_else(|| UriError::MissingSeparator(s.to_string()))?;
        if !BUILTIN_SCHEMES.contains(&scheme) {
            return Err(UriError::UnknownScheme {
                scheme: scheme.to_string(),
                known: BUILTIN_SCHEMES.iter().map(|s| s.to_string()).collect(),
            });
        }
        Ok(Uri {
            scheme: scheme.to_string(),
            path: path.to_string(),
        })
    }

    pub fn parse_or_none(s: &str) -> Option<Self> {
        Self::parse(s).ok()
    }

    pub fn is_range(value: &str) -> bool {
        value == "git://range"
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.path)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UriError {
    #[error("invalid URI (missing '://'): {0:?}")]
    MissingSeparator(String),
    #[error("unknown scheme {scheme:?}; known schemes: {known:?}")]
    UnknownScheme {
        scheme: String,
        known: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_roundtrip_file() {
        let uri = Uri::parse("file://session/foo.md").unwrap();
        assert_eq!(uri.scheme, "file");
        assert_eq!(uri.path, "session/foo.md");
        assert_eq!(uri.to_string(), "file://session/foo.md");
    }

    #[test]
    fn test_parse_roundtrip_git_range() {
        let uri = Uri::parse("git://range/abc..def").unwrap();
        assert_eq!(uri.to_string(), "git://range/abc..def");
    }

    #[test]
    fn test_parse_roundtrip_git_ref() {
        let uri = Uri::parse("git://ref/main").unwrap();
        assert_eq!(uri.to_string(), "git://ref/main");
    }

    #[test]
    fn test_parse_roundtrip_git_commit() {
        let uri = Uri::parse("git://commit/abc123").unwrap();
        assert_eq!(uri.to_string(), "git://commit/abc123");
    }

    #[test]
    fn test_parse_roundtrip_gh_pr() {
        let uri = Uri::parse("gh://pr/42").unwrap();
        assert_eq!(uri.to_string(), "gh://pr/42");
    }

    #[test]
    fn test_parse_roundtrip_gh_issue() {
        let uri = Uri::parse("gh://issue/7").unwrap();
        assert_eq!(uri.to_string(), "gh://issue/7");
    }

    #[test]
    fn test_parse_unknown_scheme() {
        let r = Uri::parse("unknown://foo");
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.to_string().contains("unknown"));
    }

    #[test]
    fn test_parse_missing_separator() {
        assert!(Uri::parse("no-slashes").is_err());
    }

    #[test]
    fn test_is_range() {
        assert!(Uri::is_range("git://range"));
        assert!(!Uri::is_range("git://commit/abc"));
        assert!(!Uri::is_range("file://foo"));
    }

    #[test]
    fn test_parse_or_none_valid() {
        let uri = Uri::parse_or_none("file://session/bar.md").unwrap();
        assert_eq!(uri.scheme, "file");
    }

    #[test]
    fn test_parse_or_none_invalid() {
        assert_eq!(Uri::parse_or_none("not-a-uri"), None);
    }

    #[test]
    fn test_display_roundtrip() {
        let uri = Uri::new("file".to_string(), "session/plan.md".to_string());
        assert_eq!(uri.to_string(), "file://session/plan.md");
    }
}