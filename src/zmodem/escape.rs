use super::frame::ZDLE;

/// ZDLE escape table: determines how each byte value is encoded when sent.
/// - 0: send as-is
/// - 1: must escape (ZDLE + byte ^ 0x40)
/// - 2: escape only if previous byte was '@' (Telenet escape prevention)
#[derive(Clone)]
pub struct EscapeTable {
    table: [u8; 256],
}

impl EscapeTable {
    /// Initialize escape table with standard ZModem escaping rules.
    /// If `escape_all_ctrl` is true, escape all control characters (ESCCTL mode).
    /// If `turbo` is true, reduce escaping for throughput.
    pub fn new(escape_all_ctrl: bool, turbo: bool) -> Self {
        let mut table = [0u8; 256];

        // Always escape ZDLE itself
        table[ZDLE as usize] = 1;

        // Always escape XON, XOFF (and their high variants)
        table[0x11] = 1; // XON
        table[0x91] = 1; // XON | 0x80
        table[0x13] = 1; // XOFF
        table[0x93] = 1; // XOFF | 0x80

        if !turbo {
            // Escape DLE and its high variant
            table[0x10] = 1; // DLE
            table[0x90] = 1; // DLE | 0x80
        }

        // CR after @ needs conditional escaping (Telenet escape sequence)
        table[b'\r' as usize] = 2;
        table[(b'\r' | 0x80) as usize] = 2;

        if escape_all_ctrl {
            // Escape all control characters (0x00-0x1f and 0x80-0x9f)
            for i in 0..0x20u8 {
                table[i as usize] = 1;
                table[(i | 0x80) as usize] = 1;
            }
        }

        Self { table }
    }

    /// Check if a byte needs escaping given the previously sent byte.
    #[inline]
    pub fn needs_escape(&self, byte: u8, last_sent: u8) -> bool {
        match self.table[byte as usize] {
            0 => false,
            1 => true,
            2 => (last_sent & 0x7f) == b'@',
            _ => unreachable!(),
        }
    }

    /// Encode a single byte, writing to the output buffer.
    /// Returns the number of bytes written (1 or 2).
    #[inline]
    pub fn encode(&self, byte: u8, last_sent: u8, out: &mut [u8; 2]) -> (usize, u8) {
        if self.needs_escape(byte, last_sent) {
            out[0] = ZDLE;
            out[1] = byte ^ 0x40;
            (2, byte ^ 0x40)
        } else {
            out[0] = byte;
            (1, byte)
        }
    }
}
