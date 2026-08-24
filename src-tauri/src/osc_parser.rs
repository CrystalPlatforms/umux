// OscParser — pure, stateful byte-stream parser (Phase 12 / #13).
//
// Deep module: a tiny push(bytes) -> PushResult surface hiding a state machine
// that recognizes AI-CLI completion notifications (OSC 9 / 99 / 777) and passes
// every other byte through untouched. No I/O — trivially unit-testable with
// fixed byte fixtures.
//
// Assumptions encoded by these tests:
//  - Input:  raw PTY bytes, fed in arbitrary-sized chunks via push().
//  - Recognized OSC command prefixes (the byte form after `ESC ]`):
//      `9;`            -> iTerm2 notification        (title "", body = message)
//      `99;`           -> Kitty notification         (best-effort: body = text
//                                                       after the 2nd `;`;
//                                                       key=value metadata,
//                                                       base64 and multi-part
//                                                       id-matching are deferred
//                                                       to NotificationService)
//      `777;notify;`   -> urxvt notification         (title + body, `;`-split)
//  - Output: PushResult { passthrough: bytes NOT belonging to a recognized
//    notification (byte-identical to input), events: one per recognized seq }.
//  - Terminators accepted: BEL (0x07) and ST (`ESC \`, 0x1B 0x5C).
//  - ConEmu/cwd disambiguation: `9;9;<cwd>` (workspace/cwd tracking emitted
//    by shell integrations) is NOT a notification — payloads whose message
//    starts with `9;` are passed through untouched. Real AI-CLI completions
//    carry a human message that never starts with `9;`. Without this, every
//    cwd update fired a bogus completion -> phantom Needs Input + a decay
//    cycle in the status dot (v0.2 Phase 2 / #26 HITL #3).
//  - Boundary: an OSC left unterminated at the end of a chunk is held until the
//    next push() supplies the terminator.
//  - NOT tested here: PtyService stream integration (separate phase),
//    NotificationService desktop delivery.

/// Which notification protocol a recognized OSC sequence used.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum OscProtocol {
    /// iTerm2: `OSC 9 ; <message>`.
    Nine,
    /// Kitty: `OSC 99 ; <metadata> ; <payload>`.
    NinetyNine,
    /// urxvt: `OSC 777 ; notify ; <title> ; <body>`.
    SevenSevenSeven,
}

/// One parsed completion notification.
#[derive(Debug, PartialEq, Clone)]
pub struct NotificationEvent {
    pub protocol: OscProtocol,
    pub title: String,
    pub body: String,
}

/// Result of feeding bytes to the parser.
#[derive(Debug, PartialEq, Default, Clone)]
pub struct PushResult {
    /// Bytes that are not part of any recognized notification, in order.
    pub passthrough: Vec<u8>,
    /// Notifications recognized in this chunk.
    pub events: Vec<NotificationEvent>,
}

/// Stateful OSC parser. Feed it PTY bytes; collect passthrough + events.
pub struct OscParser {
    // Bytes of the OSC candidate currently being collected (from `ESC ]` up to
    // but not including the terminator). Empty while in the ground state.
    buf: Vec<u8>,
    // True once we've seen `ESC ]` and are inside an OSC string.
    in_osc: bool,
    // True if the previous byte inside an OSC was ESC (start of ST terminator).
    osc_esc: bool,
    // Ground-state sub-state: we saw an ESC and are waiting for the next byte
    // to decide whether this is OSC (`ESC ]`) or some other escape (e.g. CSI
    // `ESC [`). `buf` holds the lone ESC in this state.
    after_esc: bool,
}

impl Default for OscParser {
    fn default() -> Self {
        Self::new()
    }
}

impl OscParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            in_osc: false,
            osc_esc: false,
            after_esc: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> PushResult {
        let mut result = PushResult::default();

        for &b in bytes {
            // Handle an in-flight OSC string first.
            if self.in_osc {
                if self.osc_esc {
                    // Previous byte was ESC inside the OSC.
                    self.osc_esc = false;
                    if b == b'\\' {
                        // ST terminator (ESC \) -> finalize the OSC.
                        self.finalize_osc(&[0x1b, b'\\'], &mut result);
                        continue;
                    }
                    // ESC not followed by `\`: not a terminator. Keep the ESC
                    // in the buffer and keep collecting.
                    self.buf.push(0x1b);
                    self.buf.push(b);
                    continue;
                }

                if b == 0x1b {
                    // Possible start of ST; wait for the next byte.
                    self.osc_esc = true;
                    continue;
                }

                if b == 0x07 {
                    // BEL terminator -> finalize the OSC.
                    self.finalize_osc(&[0x07], &mut result);
                    continue;
                }

                // Ordinary OSC byte.
                self.buf.push(b);
                continue;
            }

            // Ground state. An ESC could start OSC, CSI, or other sequences —
            // only `ESC ]` begins an OSC, so we wait one byte to find out.
            if self.after_esc {
                self.after_esc = false;
                if b == b']' {
                    // It's an OSC. buf already holds ESC; add the `]`.
                    self.in_osc = true;
                    self.buf.push(b']');
                    continue;
                }
                // Not an OSC: the ESC is a passthrough byte. Emit it, then
                // re-handle this byte as ground (it may itself be an ESC).
                result.passthrough.push(0x1b);
                if b == 0x1b {
                    self.after_esc = true;
                    self.buf.clear();
                    self.buf.push(0x1b);
                } else {
                    result.passthrough.push(b);
                }
                continue;
            }

            if b == 0x1b {
                // Possible escape; wait for the next byte.
                self.after_esc = true;
                self.buf.clear();
                self.buf.push(0x1b);
                continue;
            }

            result.passthrough.push(b);
        }

        result
    }

    /// Called when an OSC terminator (BEL or ST) is reached. `terminator` is the
    /// raw terminator bytes that were consumed (so a non-matching sequence can be
    /// passed through byte-identical, terminator included).
    fn finalize_osc(&mut self, terminator: &[u8], result: &mut PushResult) {
        // `buf` currently holds `ESC ] <params...>`.
        let params: &[u8] = if self.buf.len() >= 2 {
            &self.buf[2..]
        } else {
            &[]
        };

        if let Some(event) = match_notification(params) {
            result.events.push(event);
        } else {
            // Not a recognized notification: pass the whole sequence through
            // unchanged — leading `ESC ]`, params, AND the terminator — so the
            // terminal output is never altered.
            result.passthrough.extend_from_slice(&self.buf);
            result.passthrough.extend_from_slice(terminator);
        }

        self.in_osc = false;
        self.osc_esc = false;
        self.buf.clear();
    }
}

/// Inspect the OSC parameter bytes (everything between `ESC ]` and terminator)
/// and return a notification event if this is a recognized protocol.
fn match_notification(params: &[u8]) -> Option<NotificationEvent> {
    // iTerm2: `9;<message>` — but NOT `9;9;<cwd>` (ConEmu-style cwd tracking
    // that shell integrations emit on every prompt; see header). The message
    // of a real completion never starts with `9;`.
    if let Some(rest) = params.strip_prefix(b"9;") {
        if !rest.starts_with(b"9;") {
            return Some(NotificationEvent {
            protocol: OscProtocol::Nine,
            title: String::new(),
            body: String::from_utf8_lossy(rest).into_owned(),
            });
        }
    }

    // urxvt: `777;notify;<title>;<body>`. Only the `notify` extension is a
    // notification; other 777 extensions pass through.
    if let Some(rest) = params.strip_prefix(b"777;notify;") {
        let (title, body) = split_once_byte(rest, b';');
        return Some(NotificationEvent {
            protocol: OscProtocol::SevenSevenSeven,
            title: String::from_utf8_lossy(title).into_owned(),
            body: String::from_utf8_lossy(body).into_owned(),
        });
    }

    // Kitty: `99;<metadata>;<payload>`. Best-effort: body = text after the 2nd
    // `;`; the metadata block (key=value) is discarded here.
    if let Some(rest) = params.strip_prefix(b"99;") {
        let (_meta, payload) = split_once_byte(rest, b';');
        return Some(NotificationEvent {
            protocol: OscProtocol::NinetyNine,
            title: String::new(),
            body: String::from_utf8_lossy(payload).into_owned(),
        });
    }
    None
}

/// Split on the first occurrence of `sep`: returns (before, after). If `sep` is
/// absent, after is empty and before is the whole slice.
fn split_once_byte(bytes: &[u8], sep: u8) -> (&[u8], &[u8]) {
    match bytes.iter().position(|&b| b == sep) {
        Some(i) => (&bytes[..i], &bytes[i + 1..]),
        None => (bytes, &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T1 (tracer — AC1: OSC 9 emits a notification event):
    //   Input:  ESC ] 9 ; Hello BEL   (iTerm2-style completion signal)
    //   Output: one NotificationEvent { Nine, title "", body "Hello" };
    //           nothing in passthrough.
    #[test]
    fn osc9_with_bel_emits_notification() {
        let mut p = OscParser::new();
        let input = b"\x1b]9;Hello\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, Vec::<u8>::new());
        assert_eq!(
            result.events,
            vec![NotificationEvent {
                protocol: OscProtocol::Nine,
                title: String::new(),
                body: "Hello".to_string(),
            }]
        );
    }

    // T2 (AC1 — OSC 777 urxvt notify emits a notification with title + body):
    //   Input:  ESC ] 777 ; notify ; Title ; Body BEL
    //   Output: NotificationEvent { SevenSevenSeven, title "Title", body "Body" }.
    #[test]
    fn osc777_notify_emits_notification() {
        let mut p = OscParser::new();
        let input = b"\x1b]777;notify;Title;Body\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, Vec::<u8>::new());
        assert_eq!(
            result.events,
            vec![NotificationEvent {
                protocol: OscProtocol::SevenSevenSeven,
                title: "Title".to_string(),
                body: "Body".to_string(),
            }]
        );
    }

    // T3 (AC1 — OSC 99 Kitty notify emits a notification):
    //   Input:  ESC ] 99 ; <metadata> ; <payload> BEL
    //           e.g. ESC ] 99 ; i=1:p=body ; done! BEL
    //   Output: NotificationEvent { NinetyNine, title "", body "done!" }.
    //   Best-effort: body is the text after the 2nd `;`; key=value metadata,
    //   base64 and multi-part id-matching are deferred to NotificationService.
    #[test]
    fn osc99_emits_notification() {
        let mut p = OscParser::new();
        let input = b"\x1b]99;i=1:p=body;done!\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, Vec::<u8>::new());
        assert_eq!(
            result.events,
            vec![NotificationEvent {
                protocol: OscProtocol::NinetyNine,
                title: String::new(),
                body: "done!".to_string(),
            }]
        );
    }

    // T4 (AC1 — ST terminator (ESC \) works just like BEL):
    //   Input:  ESC ] 9 ; Hello ESC \
    //   Output: NotificationEvent { Nine, body "Hello" }.
    #[test]
    fn osc9_with_st_terminator_emits_notification() {
        let mut p = OscParser::new();
        let input = b"\x1b]9;Hello\x1b\\";

        let result = p.push(input);

        assert_eq!(result.passthrough, Vec::<u8>::new());
        assert_eq!(
            result.events,
            vec![NotificationEvent {
                protocol: OscProtocol::Nine,
                title: String::new(),
                body: "Hello".to_string(),
            }]
        );
    }

    // T5 (AC2 — non-matching plain text passes through unchanged, no events):
    //   Input:  "hello world"
    //   Output: passthrough "hello world", no events.
    #[test]
    fn plain_text_passes_through() {
        let mut p = OscParser::new();
        let input = b"hello world";

        let result = p.push(input);

        assert_eq!(result.passthrough, b"hello world");
        assert!(result.events.is_empty());
    }

    // T6 (AC2 — a non-notification OSC passes through byte-identical, terminator
    // included; the OSC parser must never alter terminal output):
    //   Input:  ESC ] 0 ; my title BEL   (set-window-title, not a notification)
    //   Output: passthrough == input exactly, no events.
    #[test]
    fn non_notification_osc_passes_through_with_terminator() {
        let mut p = OscParser::new();
        let input = b"\x1b]0;my title\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, input.as_slice());
        assert!(result.events.is_empty());
    }

    // T7 (AC2 — a CSI color sequence (ESC [ ...) passes through byte-identical;
    // ESC starts many sequences, only ESC ] is OSC):
    //   Input:  ESC [ 31 m red ESC [ 0 m
    //   Output: passthrough == input exactly, no events.
    #[test]
    fn csi_sequence_passes_through() {
        let mut p = OscParser::new();
        let input = b"\x1b[31mred\x1b[0m";

        let result = p.push(input);

        assert_eq!(result.passthrough, input.as_slice());
        assert!(result.events.is_empty());
    }

    // T8 (AC3 — a sequence split across chunk boundaries is reassembled):
    //   Input:  push("before" + "ESC ] 9 ; Hel")  then  push("lo BEL after")
    //   Output: passthrough "beforeafter", one event { Nine, body "Hello" }.
    //   The OSC candidate is held across the two push() calls.
    #[test]
    fn osc9_split_across_chunks_is_reassembled() {
        let mut p = OscParser::new();

        let r1 = p.push(b"before\x1b]9;Hel");
        // Surrounding text "before" passes through immediately; the OSC is
        // mid-collection, so no event yet.
        assert_eq!(r1.passthrough, b"before");
        assert!(r1.events.is_empty());

        let r2 = p.push(b"lo\x07after");
        assert_eq!(r2.passthrough, b"after");
        assert_eq!(
            r2.events,
            vec![NotificationEvent {
                protocol: OscProtocol::Nine,
                title: String::new(),
                body: "Hello".to_string(),
            }]
        );
    }

    // T9 (AC3 — the terminator itself can land in a later chunk):
    //   Input:  push("ESC ] 9 ; Hello")  then  push("BEL")
    //   Output: event { Nine, body "Hello" } emitted on the second push.
    #[test]
    fn terminator_bel_in_separate_chunk() {
        let mut p = OscParser::new();

        let r1 = p.push(b"\x1b]9;Hello");
        assert!(r1.passthrough.is_empty());
        assert!(r1.events.is_empty());

        let r2 = p.push(b"\x07");
        assert!(r2.passthrough.is_empty());
        assert_eq!(
            r2.events,
            vec![NotificationEvent {
                protocol: OscProtocol::Nine,
                title: String::new(),
                body: "Hello".to_string(),
            }]
        );
    }

    // T10 (AC3 — the two-byte ST terminator can straddle a chunk boundary):
    //   Input:  push("ESC ] 9 ; Hello ESC")  then  push("\")
    //   Output: event { Nine, body "Hello" } on the second push.
    #[test]
    fn terminator_st_split_across_chunks() {
        let mut p = OscParser::new();

        let r1 = p.push(b"\x1b]9;Hello\x1b");
        assert!(r1.passthrough.is_empty());
        assert!(r1.events.is_empty());

        let r2 = p.push(b"\\");
        assert!(r2.passthrough.is_empty());
        assert_eq!(
            r2.events,
            vec![NotificationEvent {
                protocol: OscProtocol::Nine,
                title: String::new(),
                body: "Hello".to_string(),
            }]
        );
    }

    // T11 (AC1+AC2 — multiple notifications and surrounding text in one stream):
    //   Input:  "a" ESC ] 9 ; one BEL "b" ESC ] 777 ; notify ; T ; two BEL "c"
    //   Output: passthrough "abc", two events in order.
    #[test]
    fn multiple_notifications_and_text_interleaved() {
        let mut p = OscParser::new();
        let input = b"a\x1b]9;one\x07b\x1b]777;notify;T;two\x07c";

        let result = p.push(input);

        assert_eq!(result.passthrough, b"abc");
        assert_eq!(
            result.events,
            vec![
                NotificationEvent {
                    protocol: OscProtocol::Nine,
                    title: String::new(),
                    body: "one".to_string(),
                },
                NotificationEvent {
                    protocol: OscProtocol::SevenSevenSeven,
                    title: "T".to_string(),
                    body: "two".to_string(),
                },
            ]
        );
    }

    // T13 (v0.2 Phase 2 / #26 HITL #3 — cwd tracking is not a completion):
    //   Input:  ESC ] 9 ; 9 ; /Users/adam/project BEL
    //           (ConEmu-style cwd sequence that shell integrations emit on
    //           every prompt; without the guard every cwd update fired a
    //           bogus Needs Input + desktop notification)
    //   Output: passthrough == input exactly, no events.
    #[test]
    fn osc_9_9_cwd_tracking_is_not_a_notification() {
        let mut p = OscParser::new();
        let input = b"\x1b]9;9;/Users/adam/project\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, input.as_slice());
        assert!(result.events.is_empty());
    }

    // T12 (AC2 — OSC 777 carries other extensions too; only `notify` is a
    // notification. A non-notify 777 passes through byte-identical, no event):
    //   Input:  ESC ] 777 ; other ; x BEL
    //   Output: passthrough == input, no events.
    #[test]
    fn osc777_without_notify_passes_through() {
        let mut p = OscParser::new();
        let input = b"\x1b]777;other;x\x07";

        let result = p.push(input);

        assert_eq!(result.passthrough, input.as_slice());
        assert!(result.events.is_empty());
    }
}
