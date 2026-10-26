// queries — replay must not re-interrogate the terminal (#120).
//
// A terminal-interrogation sequence (the host asking the TERMINAL to answer
// on its input stream) is safe exactly once, on the live terminal that was
// there when it was sent: the answer goes to whatever program is reading.
// The scrollback ring keeps the query bytes verbatim, so a REPLAY delivers
// them again — into a fresh xterm bound to the same session. xterm answers
// (a primary-DA probe `\x1b[c` is answered with `\x1b[?1;2c`), the answer
// lands in the session's input, and a shell idling at a prompt echoes it as
// literal text: `1;2c1;2c…` — one more run on every reattach/restart, the
// macOS restart garbage of issue #120 (the #115 boundary fix only stopped
// replaying HEADLESS sequence tails; an INTACT query replays fine and still
// re-triggers).
//
// The replay stream is therefore a reconstruction, not verbatim history:
// `strip_device_queries` copies the bytes through UNCHANGED except the
// interrogation sequences, which it drops whole. The LIVE stream (ring
// capture, subscriber fan-out) stays byte-identical — the byte policy is
// untouched; only the replay path filters, exactly where #115 already
// deviates (the window realignment also drops bytes).
//
// Pure: no I/O. Same scanner philosophy as `escape_boundary` — just enough
// state to know where sequences begin and end, never what they mean beyond
// the drop decision. 8-bit C1 bytes are continuation bytes in this UTF-8
// stream, never CSI openers.
//
// Dropped classes (everything a fresh xterm.js answers, plus the echoed
// answers that re-trigger the same response):
// - `CSI [prefix] c` with prefix none-or-`>` and no params or params[0]==0
//   (primary/secondary DA probes — xterm.js answers exactly these; the
//   echoed secondary answer `\x1b[>0;…;0c` carries params[0]==0 too and
//   would re-trigger, so it drops under the same rule),
// - `CSI [prefix] n` (DSR status / CPR / DECXCPR — final `n` has no
//   non-query meaning),
// - `CSI … $ p` (DECRQM mode query),
// - DCS strings introduced by `$`, `+` or `!` (DECRQSS, XTGETTCAP, DA3
//   request) — dropped through the terminator; other DCS payloads (Sixel)
//   pass,
// - OSC strings whose last `;`-separated field is exactly `?` (color /
//   clipboard / font queries) — dropped through BEL or ST; every other
//   OSC (9;9 included) passes.
//
// Aborted strings (a fresh ESC inside CSI/string) pass through verbatim: a
// fresh terminal applies its own abort rules to them — and a query without
// its terminator dispatches nothing, so verbatim is inert and never drops
// display bytes.

/// Copy `bytes` through, dropping the terminal-interrogation sequences.
pub fn strip_device_queries(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut state = Scanner::Ground;
    for &b in bytes {
        match state {
            Scanner::Ground => {
                if b == 0x1b {
                    state = Scanner::Esc;
                } else {
                    out.push(b);
                }
            }
            Scanner::Esc => {
                state = match b {
                    b'[' => Scanner::Csi(Csi::started()),
                    b'P' => Scanner::Str(Str::dcs()),
                    b']' => Scanner::Str(Str::osc()),
                    0x1b => Scanner::Esc,
                    _ => {
                        // ESC + any other byte: an escaped C0 form — pass both.
                        out.push(0x1b);
                        out.push(b);
                        Scanner::Ground
                    }
                };
            }
            Scanner::Csi(mut seq) => {
                if b == 0x1b {
                    // Aborted CSI: the bytes were never a complete sequence —
                    // pass them and let this ESC start the next decision.
                    out.extend_from_slice(&seq.buf);
                    out.push(b);
                    state = Scanner::Esc;
                } else if (0x40..=0x7e).contains(&b) {
                    // Final byte: the one place a drop is decided.
                    seq.observe(b);
                    seq.buf.push(b);
                    if !seq.is_interrogation(b) {
                        out.extend_from_slice(&seq.buf);
                    }
                    state = Scanner::Ground;
                } else {
                    seq.observe(b);
                    seq.buf.push(b);
                    state = Scanner::Csi(seq);
                }
            }
            Scanner::Str(mut seq) => {
                seq.buf.push(b);
                let prev_was_esc = seq.buf.len() >= 2 && seq.buf[seq.buf.len() - 2] == 0x1b;
                if b == 0x07 || (prev_was_esc && b == b'\\') {
                    // Terminated (BEL or ST): the only path that reaches the
                    // terminal's dispatcher — decide on the content.
                    if !seq.is_query() {
                        out.extend_from_slice(&seq.buf);
                    }
                    state = Scanner::Ground;
                } else if prev_was_esc {
                    // Aborted (byte after ESC is not `\`): pass verbatim —
                    // inert, and never over-drop.
                    out.extend_from_slice(&seq.buf);
                    state = Scanner::Ground;
                } else {
                    state = Scanner::Str(seq);
                }
            }
        }
    }
    // A stream ending mid-sequence: whatever is buffered never completed —
    // pass it (inert on a fresh terminal, same rule as aborts). An
    // unterminated buffer is exactly what the escape_boundary scan prevents
    // the replay window from ENDING inside, so this is belt-and-braces.
    match state {
        Scanner::Esc => out.push(0x1b),
        Scanner::Csi(seq) => out.extend_from_slice(&seq.buf),
        Scanner::Str(seq) => out.extend_from_slice(&seq.buf),
        Scanner::Ground => {}
    }
    out
}

/// Where the scanner sits. `Ground` means bytes flow straight through.
enum Scanner {
    Ground,
    Esc,
    Csi(Csi),
    Str(Str),
}

/// CSI bookkeeping: the buffered bytes plus only the facts the drop
/// decision needs (prefix byte, first param value, a `$` intermediate).
struct Csi {
    buf: Vec<u8>,
    /// True until the first byte after `ESC [` is absorbed — that byte is
    /// the private-marker position.
    after_intro: bool,
    /// The private-marker byte (`<` `=` `>` `?`) when the sequence opened
    /// with one — always the byte right after `ESC [`.
    prefix: Option<u8>,
    /// The first numeric param, once its leading digits have been read
    /// (None = no param yet). Parsing STOPS at the first `;` — later
    /// params must never bleed into it (`CSI > 0 ; 276 ; 0 c` is a
    /// params[0]==0 sequence, not a 2760 one). Saturates at u32::MAX; only
    /// the ==0 test matters.
    first_param: Option<u32>,
    /// The first param's digits are over (a `;` arrived).
    first_param_done: bool,
    /// A `$` intermediate byte was seen (DECRQM's marker).
    dollar: bool,
}

impl Csi {
    fn started() -> Self {
        Csi {
            // The introducers ride in the buffer: pass-through must emit
            // them (only the drop path skips the flush).
            buf: vec![0x1b, b'['],
            after_intro: true,
            prefix: None,
            first_param: None,
            first_param_done: false,
            dollar: false,
        }
    }

    /// Absorb one byte's facts (the byte itself is pushed by the caller).
    fn observe(&mut self, b: u8) {
        if self.after_intro {
            self.after_intro = false;
            // The byte right after `ESC [`: a private marker or a param.
            if (0x3c..=0x3f).contains(&b) {
                self.prefix = Some(b);
            }
        }
        if b == b';' {
            self.first_param_done = true;
        } else if b.is_ascii_digit() && !self.first_param_done {
            let digit = u32::from(b - b'0');
            self.first_param = Some(match self.first_param {
                Some(p) => p.saturating_mul(10).saturating_add(digit),
                None => digit,
            });
        }
        if b == b'$' {
            self.dollar = true;
        }
    }

    /// The drop rule for a CSI whose final byte is `final`.
    fn is_interrogation(&self, final_byte: u8) -> bool {
        match final_byte {
            b'c' => {
                // DA probes: xterm.js answers when prefix is absent or `>`
                // AND the first param is absent or 0. The echoed secondary
                // answer (`CSI > 0 ; … ; 0 c`) satisfies the same test and
                // re-triggers the same answer — dropped too, deliberately.
                matches!(self.prefix, None | Some(b'>'))
                    && self.first_param.unwrap_or(0) == 0
            }
            b'n' => true, // DSR / CPR / DECXCPR: `n` is always a request
            b'p' => self.dollar, // DECRQM (`CSI Ps $ p`, `CSI ? Ps $ p`)
            _ => false,
        }
    }
}

/// A string sequence (OSC / DCS): buffer the body, end at BEL or ST.
struct Str {
    buf: Vec<u8>,
    /// True for OSC (introducer `ESC ]`), false for DCS (`ESC P`).
    osc: bool,
}

impl Str {
    fn osc() -> Self {
        Str {
            buf: vec![0x1b, b']'],
            osc: true,
        }
    }

    fn dcs() -> Self {
        Str {
            buf: vec![0x1b, b'P'],
            osc: false,
        }
    }

    /// The drop rule. OSC: the LAST `;`-separated field of the body is
    /// exactly `?` (color/clipboard/font queries). DCS: the FIRST body
    /// byte names the query families `$` (DECRQSS), `+` (XTGETTCAP), `!`
    /// (DA3 request). The introducer occupies buf's first two bytes; the
    /// body starts after it.
    fn is_query(&self) -> bool {
        if self.osc {
            let body = strip_terminator(&self.buf[2..]);
            match body.rsplit(|&b| b == b';').next() {
                Some(field) => field == b"?",
                None => false,
            }
        } else {
            self.buf
                .get(2)
                .is_some_and(|first| matches!(first, b'$' | b'+' | b'!'))
        }
    }
}

/// Drop a trailing BEL or ST from a buffered string body, so a field
/// comparison never sees the terminator bytes.
fn strip_terminator(buf: &[u8]) -> &[u8] {
    if buf.ends_with(&[0x1b, b'\\']) {
        &buf[..buf.len() - 2]
    } else if buf.ends_with(&[0x07]) {
        &buf[..buf.len() - 1]
    } else {
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The #120 reproduction: an INTACT primary-DA probe sits in the ring
    // (a program wrote `\x1b[c`; the live xterm answered it long ago). A
    // fresh terminal replaying it would ANSWER into the shell — echoed as
    // visible `1;2c` on every reattach. The strip drops the query whole.
    #[test]
    fn intact_da1_query_drops_whole() {
        let out = strip_device_queries(b"prompt> \x1b[c");
        assert_eq!(out, b"prompt> ");
    }

    // The zero-param spelling and the explicit-zero spelling query too —
    // xterm.js answers both (params[0] absent or 0).
    #[test]
    fn zero_param_da_spelling_drops() {
        for q in [&b"\x1b[0c"[..], b"\x1b[00c"] {
            assert_eq!(strip_device_queries(q), b"", "{:?} is a query", q);
        }
    }

    // The echoed PRIMARY answer (`\x1b[?1;2c`, the tty echo of xterm's
    // input) must SURVIVE: xterm.js ignores it (params[0]=1 > 0 → no
    // response) and it is honest recorded history — dropping display-safe
    // bytes would over-strip.
    #[test]
    fn echoed_primary_answer_survives() {
        assert_eq!(strip_device_queries(b"\x1b[?1;2c"), b"\x1b[?1;2c");
    }

    // The echoed SECONDARY answer re-triggers (xterm.js answers any
    // `CSI > c` whose params[0]==0) — dropped under the same rule as the
    // query itself. The `;` must stop first-param parsing: with params
    // bleeding across `;` this byte string misreads as params[0]==2760.
    #[test]
    fn echoed_secondary_answer_drops_on_first_param_zero() {
        assert_eq!(strip_device_queries(b"\x1b[>0;276;0c"), b"");
    }

    // A secondary DA whose first param is nonzero gets NO answer from
    // xterm.js — it survives.
    #[test]
    fn secondary_da_with_first_param_survives() {
        assert_eq!(strip_device_queries(b"\x1b[>1;276;0c"), b"\x1b[>1;276;0c");
    }

    // DSR / CPR / DECXCPR: final `n` is always an interrogation.
    #[test]
    fn dsr_and_cpr_forms_drop() {
        for q in [&b"\x1b[5n"[..], b"\x1b[6n", b"\x1b[?6n", b"\x1b[0n"] {
            assert_eq!(strip_device_queries(q), b"", "{:?} is a query", q);
        }
    }

    // DECRQM (`CSI ? 2026 $ p` — the synced-output mode query) drops; its
    // ANSWER (`CSI ? 2026 ; 2 $ y`) survives (final `y` is unknown here).
    #[test]
    fn decrqm_drops_but_its_answer_survives() {
        assert_eq!(strip_device_queries(b"\x1b[?2026$p"), b"");
        assert_eq!(
            strip_device_queries(b"\x1b[?2026;2$y"),
            b"\x1b[?2026;2$y"
        );
    }

    // XTGETTCAP: the query (`DCS + q … ST`) drops whole; the answer
    // (`DCS 0 + r … ST`) survives — its first body byte is `0`.
    #[test]
    fn xtgettcap_query_drops_answer_survives() {
        assert_eq!(strip_device_queries(b"\x1bP+q544554431b\\\x1b\\"), b"");
        assert_eq!(
            strip_device_queries(b"\x1bP0+r544554431b\\\x1b\\"),
            b"\x1bP0+r544554431b\\\x1b\\"
        );
    }

    // Sixel graphics ride DCS with a digit-led body — never dropped.
    #[test]
    fn sixel_dcs_survives() {
        let sixel = b"\x1bP0;1q#0;2;0;0;0-\x1b\\";
        assert_eq!(strip_device_queries(sixel), &sixel[..]);
    }

    // OSC color query drops; OSC 9;9 (umux's own completion cwd signal)
    // and a color ANSWER survive verbatim.
    #[test]
    fn osc_query_drops_own_osc_survives() {
        assert_eq!(strip_device_queries(b"\x1b]11;?\x07"), b"");
        assert_eq!(
            strip_device_queries(b"\x1b]11;rgb:0000/0000/0000\x07"),
            b"\x1b]11;rgb:0000/0000/0000\x07"
        );
        let osc99 = b"\x1b]9;9;/home/dev/umux\x07";
        assert_eq!(strip_device_queries(osc99), &osc99[..]);
    }

    // The ST-terminated OSC query spelling (`OSC 11 ; ? ST`) drops too.
    #[test]
    fn st_terminated_osc_query_drops() {
        assert_eq!(strip_device_queries(b"\x1b]11;?\x1b\\"), b"");
    }

    // Byte identity for ordinary output: a prompt with SGR coloring, a
    // cursor show and an OSC title pass through unchanged.
    #[test]
    fn ordinary_output_is_byte_identical() {
        let chunk: &[u8] = b"\x1b]2;umux\x07\x1b[1m$ \x1b[0m\x1b[?25hls\r\n";
        assert_eq!(strip_device_queries(chunk), chunk);
    }

    // An OSC aborted by a fresh ESC passes verbatim — the fresh terminal
    // applies its own abort rules (`\x1b[1m` renders bold), and an
    // unterminated query dispatches nothing.
    #[test]
    fn aborted_osc_passes_verbatim() {
        assert_eq!(
            strip_device_queries(b"\x1b]0;title\x1b[1mbold"),
            b"\x1b]0;title\x1b[1mbold"
        );
    }

    // A query aborted before its terminator is inert — verbatim is fine.
    #[test]
    fn aborted_csi_passes_verbatim() {
        assert_eq!(strip_device_queries(b"\x1b[6"), b"\x1b[6");
    }

    // A stream ending on a bare ESC flushes it.
    #[test]
    fn trailing_esc_flushes() {
        assert_eq!(strip_device_queries(b"ok\x1b"), b"ok\x1b");
    }

    // The exact replay shape of the bug: an aligned window holding a
    // prompt, an intact DA query and the echoed answer loses ONLY the
    // query — the answer (harmless) and the prompt stay.
    #[test]
    fn replay_slice_shaped_mixed_stream_keeps_everything_but_queries() {
        let input: &[u8] = b"$ vim notes.md\x1b[cmore text\x1b[?1;2c\x1b]9;9;/p\x07";
        let expected: &[u8] = b"$ vim notes.mdmore text\x1b[?1;2c\x1b]9;9;/p\x07";
        assert_eq!(strip_device_queries(input), expected);
    }
}
