//! Byte-span types for source-accurate positioning.

/// Half-open byte range `[start, end)` into the original source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SourceSpan {
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

impl SourceSpan {
    /// Create a new span.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the span covers zero bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Line ending style detected from the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n` — Unix, Linux, modern macOS.
    Lf,
    /// `\r\n` — Windows.
    Crlf,
    /// `\r` — classic Mac OS.
    Cr,
}

impl LineEnding {
    /// Detect the dominant line ending from source bytes.
    ///
    /// Returns [`LineEnding::Lf`] when no line endings are present (including
    /// the empty string).
    #[must_use]
    pub fn detect(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut lf = 0usize;
        let mut crlf = 0usize;
        let mut cr = 0usize;

        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    crlf += 1;
                    i += 2;
                    continue;
                }
                cr += 1;
            } else if bytes[i] == b'\n' {
                lf += 1;
            }
            i += 1;
        }

        if crlf >= lf && crlf >= cr && crlf > 0 {
            LineEnding::Crlf
        } else if cr > lf && cr > 0 {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_lf() {
        assert_eq!(LineEnding::detect("a\nb\nc\n"), LineEnding::Lf);
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\r\n"), LineEnding::Crlf);
    }

    #[test]
    fn detect_cr() {
        assert_eq!(LineEnding::detect("a\rb\rc\r"), LineEnding::Cr);
    }

    #[test]
    fn detect_empty_defaults_lf() {
        assert_eq!(LineEnding::detect(""), LineEnding::Lf);
    }

    #[test]
    fn detect_mixed_prefers_majority() {
        // 2 CRLF vs 1 LF → CRLF
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\n"), LineEnding::Crlf);
        // 2 LF vs 1 CRLF → LF
        assert_eq!(LineEnding::detect("a\nb\nc\r\n"), LineEnding::Lf);
    }

    #[test]
    fn span_len_and_empty() {
        assert_eq!(SourceSpan::new(0, 7).len(), 7);
        assert!(!SourceSpan::new(0, 7).is_empty());
        assert!(SourceSpan::new(3, 3).is_empty());
    }
}
