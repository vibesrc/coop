use std::time::Instant;

use aho_corasick::{AhoCorasick, MatchKind};

/// Input filter for web-connected PTYs.
/// Blocks dangerous sequences and debounces Ctrl+C.
pub struct InputFilter {
    /// Aho-Corasick automaton for blocked sequences
    automaton: AhoCorasick,
    /// The blocked patterns (for reference)
    patterns: Vec<Vec<u8>>,
    /// Partial match buffer
    pending: Vec<u8>,
    /// Last time Ctrl+C was forwarded
    last_ctrl_c: Option<Instant>,
    /// Debounce interval for Ctrl+C
    ctrl_c_debounce_ms: u64,
    /// Partial match timeout (ms)
    partial_timeout_ms: u64,
    /// Time of last input
    last_input: Option<Instant>,
}

/// Warning message sent when input is blocked
pub const BLOCKED_WARNING: &[u8] =
    b"\r\n\x1b[1;33m\xe2\x9a\xa0  Blocked from web. Use local terminal to exit.\x1b[0m\r\n";

/// Warning message for debounced Ctrl+C
pub const CTRL_C_DEBOUNCED_WARNING: &[u8] =
    b"\r\n\x1b[1;33m\xe2\x9a\xa0  Ctrl+C debounced. Wait before pressing again.\x1b[0m\r\n";

impl InputFilter {
    pub fn new(ctrl_c_debounce_ms: u64, custom_block_sequences: &[String]) -> Self {
        let mut patterns: Vec<Vec<u8>> = vec![
            // Built-in blocked sequences
            vec![0x04],          // Ctrl+D
            vec![0x1c],          // Ctrl+\ (SIGQUIT)
            b"exit\r".to_vec(),  // exit + enter
            b"exit\n".to_vec(),  // exit + newline
            b"/exit\r".to_vec(), // /exit + enter
            b"/exit\n".to_vec(), // /exit + newline
            b"quit\r".to_vec(),  // quit + enter
            b"quit\n".to_vec(),  // quit + newline
        ];

        // Add custom block sequences
        for seq in custom_block_sequences {
            patterns.push(parse_escape_sequence(seq));
        }

        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
            .expect("Failed to build Aho-Corasick automaton");

        Self {
            automaton,
            patterns,
            pending: Vec::new(),
            last_ctrl_c: None,
            ctrl_c_debounce_ms,
            partial_timeout_ms: 500,
            last_input: None,
        }
    }

    /// Filter input bytes. Returns (bytes_to_forward, optional_warning).
    pub fn filter(&mut self, input: &[u8]) -> (Vec<u8>, Option<&'static [u8]>) {
        let now = Instant::now();
        self.last_input = Some(now);

        // Handle Ctrl+C debouncing (single byte check before Aho-Corasick)
        if input == [0x03] {
            return self.handle_ctrl_c(now);
        }

        // Flush pending if partial match timed out
        let mut output = Vec::new();
        if !self.pending.is_empty() {
            if let Some(last) = self.last_input {
                if now.duration_since(last).as_millis() as u64 > self.partial_timeout_ms {
                    output.extend_from_slice(&self.pending);
                    self.pending.clear();
                }
            }
        }

        // Combine pending + new input for matching
        self.pending.extend_from_slice(input);

        // Check for blocked sequences
        let combined = self.pending.clone();
        let matches: Vec<_> = self.automaton.find_iter(&combined).collect();

        if matches.is_empty() {
            // No match - check if any pattern could still be a prefix match
            if self.could_be_prefix(&combined) {
                // Hold bytes, might still match
                return (output, None);
            } else {
                // No possible match, forward everything
                output.extend_from_slice(&self.pending);
                self.pending.clear();
                return (output, None);
            }
        }

        // Found a blocked sequence - suppress it
        self.pending.clear();
        (Vec::new(), Some(BLOCKED_WARNING))
    }

    fn handle_ctrl_c(&mut self, now: Instant) -> (Vec<u8>, Option<&'static [u8]>) {
        if let Some(last) = self.last_ctrl_c {
            if now.duration_since(last).as_millis() as u64 <= self.ctrl_c_debounce_ms {
                // Within debounce window, suppress
                return (Vec::new(), Some(CTRL_C_DEBOUNCED_WARNING));
            }
        }

        // Forward this Ctrl+C and start debounce timer
        self.last_ctrl_c = Some(now);
        (vec![0x03], None)
    }

    fn could_be_prefix(&self, data: &[u8]) -> bool {
        // Check if the data is a prefix of any blocked pattern
        for pattern in &self.patterns {
            if pattern.len() > data.len() && pattern.starts_with(data) {
                return true;
            }
        }
        false
    }

    /// Flush any pending bytes (call on timeout)
    #[allow(dead_code)]
    pub fn flush_pending(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

/// Parse escape sequences like \x03 into byte vectors
fn parse_escape_sequence(s: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if i + 3 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'x' {
            // Parse \xNN hex escape
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 2..i + 4]).unwrap_or(""), 16)
            {
                result.push(byte);
                i += 4;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_ctrl_d() {
        let mut filter = InputFilter::new(500, &[]);
        let (out, warning) = filter.filter(&[0x04]);
        assert!(out.is_empty());
        assert!(warning.is_some());
    }

    #[test]
    fn test_passes_normal_input() {
        let mut filter = InputFilter::new(500, &[]);
        let (out, warning) = filter.filter(b"hello");
        assert_eq!(out, b"hello");
        assert!(warning.is_none());
    }

    #[test]
    fn test_ctrl_c_debounce() {
        let mut filter = InputFilter::new(500, &[]);

        // First Ctrl+C passes through
        let (out, warning) = filter.filter(&[0x03]);
        assert_eq!(out, vec![0x03]);
        assert!(warning.is_none());

        // Immediate second Ctrl+C is debounced
        let (out, warning) = filter.filter(&[0x03]);
        assert!(out.is_empty());
        assert!(warning.is_some());
    }
}
