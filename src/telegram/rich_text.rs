//! A message's formatted text (TDLib's text and its entities) as Slint's StyledText, which is made
//! from Markdown: bold, italic, underline, strikethrough, code and links.
//!
//! The words themselves must never read as Markdown, so every ASCII punctuation character in them
//! is escaped, and each line is its own paragraph (Slint makes a new line of every line break).
//! Every styled run is written separately, its delimiters guarded by word joiners (U+2060, neither
//! space nor punctuation to CommonMark, and invisible), so that emphasis opens and closes wherever
//! it starts and ends. Slint's StyledText cannot draw block quotes: their lines start with "│ ".

use super::api::{FormattedText, TextEntityType};

/// Invisible, and to CommonMark neither whitespace nor punctuation: next to it, a delimiter run can
/// always open and close.
const GUARD: char = '\u{2060}';
/// Keeps a line's leading spaces, and an empty line, from meaning something in Markdown.
const NO_BREAK_SPACE: char = '\u{a0}';

/// A message's formatted text for Slint, or None when nothing in it is formatted or linked: the
/// pages show it as plain text then, in the design's own line spacing.
pub fn styled_text(text: &FormattedText) -> Option<slint::StyledText> {
    let markdown = markdown(text)?;
    match slint::StyledText::from_markdown(&markdown) {
        Ok(styled) => Some(styled),
        Err(err) => {
            eprintln!("rich text: showing a message as plain text: {err}");
            None
        }
    }
}

/// What an entity does to its part of the text. A link leads to a web or mail address, or to a
/// Telegram chat by its username or its user (Conversation.open-link opens those in FinchGram).
#[derive(Debug, Clone, PartialEq)]
enum Style {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Quote,
    Link(String),
}

/// The Markdown for `text`, or None when there is nothing to format.
pub fn markdown(text: &FormattedText) -> Option<String> {
    let chars: Vec<char> = text.text.chars().filter(|&c| c != '\r').collect();
    // UTF-16 offsets to character indices.
    let mut starts = Vec::with_capacity(chars.len() + 1);
    let mut units = 0usize;
    for &c in &chars {
        starts.push(units);
        units += c.len_utf16();
    }
    starts.push(units);
    let index = |unit: i32| starts.partition_point(|&start| start < unit.max(0) as usize);

    let mut spans: Vec<(usize, usize, Style)> = Vec::new();
    for entity in &text.entities {
        let (start, end) = (index(entity.offset), index(entity.offset.saturating_add(entity.length)));
        if start >= end {
            continue;
        }
        let words: String = chars[start..end].iter().collect();
        let style = match &entity.kind {
            TextEntityType::Bold => Style::Bold,
            TextEntityType::Italic => Style::Italic,
            TextEntityType::Underline => Style::Underline,
            TextEntityType::Strikethrough => Style::Strikethrough,
            TextEntityType::Code | TextEntityType::Pre | TextEntityType::PreCode {} => Style::Code,
            TextEntityType::BlockQuote | TextEntityType::ExpandableBlockQuote => Style::Quote,
            TextEntityType::Url => Style::Link(web_address(&words)),
            TextEntityType::TextUrl { url } => Style::Link(url.clone()),
            TextEntityType::EmailAddress => Style::Link(format!("mailto:{words}")),
            TextEntityType::Mention => Style::Link(format!("tg://resolve?domain={}", words.trim_start_matches('@'))),
            TextEntityType::MentionName { user_id } => Style::Link(format!("tg://user?id={user_id}")),
            TextEntityType::Other => continue,
        };
        spans.push((start, end, style));
    }
    if spans.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    let mut line_start = 0;
    for end in chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(i, _)| i).chain(std::iter::once(chars.len())) {
        lines.push(line(&chars, line_start, end, &spans));
        line_start = end + 1;
    }
    Some(lines.join("\n"))
}

/// One line of the text, from `start` to `end` (its line break not included).
fn line(chars: &[char], start: usize, end: usize, spans: &[(usize, usize, Style)]) -> String {
    let covering = |from: usize, to: usize| -> Vec<&Style> {
        spans.iter().filter(|(start, end, _)| *start <= from && to <= *end).map(|(_, _, style)| style).collect()
    };
    let mut out = String::new();
    if start < end && covering(start, start + 1).contains(&&Style::Quote) || start == end && covering(start.saturating_sub(1), start).contains(&&Style::Quote)
    {
        out.push_str("│ ");
    }
    // Leading spaces would indent the line into a code block.
    let mut words: Vec<char> = chars[start..end].to_vec();
    for c in words.iter_mut().take_while(|c| **c == ' ' || **c == '\t') {
        *c = NO_BREAK_SPACE;
    }
    if words.is_empty() {
        out.push(NO_BREAK_SPACE);
        return out;
    }
    // Runs of the same styles, each written on its own.
    let mut cuts: Vec<usize> = vec![0, words.len()];
    for (span_start, span_end, _) in spans {
        for cut in [*span_start, *span_end] {
            if cut > start && cut < end {
                cuts.push(cut - start);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut previous: Option<Vec<&Style>> = None;
    let mut run_start = 0;
    for pair in cuts.windows(2) {
        let styles = covering(start + pair[0], start + pair[1]);
        if previous.as_ref().is_some_and(|previous| *previous != styles) {
            write_run(&mut out, &words[run_start..pair[0]], previous.as_deref().unwrap_or_default());
            out.push(GUARD);
            run_start = pair[0];
        }
        previous = Some(styles);
    }
    write_run(&mut out, &words[run_start..], previous.as_deref().unwrap_or_default());
    out
}

fn write_run(out: &mut String, words: &[char], styles: &[&Style]) {
    let has = |style: &Style| styles.contains(&style);
    let mut run = if has(&Style::Code) { code_span(words) } else { escape(words) };
    let mut wrap = |before: &str, after: &str| {
        run = format!("{before}{GUARD}{run}{GUARD}{after}");
    };
    if has(&Style::Italic) {
        wrap("*", "*");
    }
    if has(&Style::Bold) {
        wrap("**", "**");
    }
    if has(&Style::Strikethrough) {
        wrap("~~", "~~");
    }
    if has(&Style::Underline) {
        run = format!("<u>{run}</u>");
    }
    if let Some(Style::Link(target)) = styles.iter().find(|style| matches!(style, Style::Link(_))) {
        run = format!("[{run}](<{}>)", destination(target));
    }
    out.push_str(&run);
}

/// The words as literal text: every ASCII punctuation character escaped.
fn escape(words: &[char]) -> String {
    let mut out = String::with_capacity(words.len() * 2);
    for &c in words {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Code as a code span: fenced by more backticks than it has in a row, and spaced when it starts or
/// ends with a backtick or a space.
fn code_span(words: &[char]) -> String {
    let code: String = words.iter().collect();
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    if code.starts_with(['`', ' ']) || code.ends_with(['`', ' ']) {
        format!("{fence} {code} {fence}")
    } else {
        format!("{fence}{code}{fence}")
    }
}

/// A link's destination between angle brackets: nothing in it may close them or escape.
fn destination(target: &str) -> String {
    target.replace('\\', "%5C").replace('<', "%3C").replace('>', "%3E").replace(' ', "%20").replace('\n', "%0A")
}

/// A web address as written in a message ("example.org/page"), with its scheme.
fn web_address(words: &str) -> String {
    if words.contains("://") { words.to_string() } else { format!("https://{words}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::api::TextEntity;

    fn text(words: &str, entities: &[(i32, i32, TextEntityType)]) -> FormattedText {
        FormattedText {
            text: words.into(),
            entities: entities.iter().map(|(offset, length, kind)| TextEntity { offset: *offset, length: *length, kind: kind.clone() }).collect(),
        }
    }

    #[test]
    fn plain_text_has_no_markdown() {
        assert_eq!(markdown(&text("just words", &[])), None);
        assert_eq!(markdown(&text("#tag", &[(0, 4, TextEntityType::Other)])), None);
    }

    #[test]
    fn styles_and_links_become_markdown() {
        let bold = markdown(&text("a bold word", &[(2, 4, TextEntityType::Bold)])).unwrap();
        assert_eq!(bold, format!("a {GUARD}**{GUARD}bold{GUARD}**{GUARD} word"));
        let link = markdown(&text("see example.org now", &[(4, 11, TextEntityType::Url)])).unwrap();
        assert_eq!(link, format!("see {GUARD}[example\\.org](<https://example.org>){GUARD} now"));
        let mention = markdown(&text("hi @finchgram", &[(3, 10, TextEntityType::Mention)])).unwrap();
        assert!(mention.contains("(<tg://resolve?domain=finchgram>)"));
    }

    #[test]
    fn offsets_count_utf16_units() {
        // 😀 is two UTF-16 units: "bold" starts at 3.
        let styled = markdown(&text("😀 bold", &[(3, 4, TextEntityType::Bold)])).unwrap();
        assert!(styled.starts_with("😀 "), "{styled}");
        assert!(styled.contains("**\u{2060}bold\u{2060}**"), "{styled}");
    }

    #[test]
    fn nothing_in_the_words_reads_as_markdown() {
        let tricky = "# heading\n> quote\n1. list\n- item\n---\n    indented\n<b>html</b> &amp; \\ [a](b) *x* _y_ `z` ~~w~~\n\n=====";
        let formatted = text(tricky, &[(0, 1, TextEntityType::Bold)]);
        let styled = styled_text(&formatted).expect("Slint reads it");
        let _ = styled;
        let written = markdown(&formatted).unwrap();
        assert!(written.contains("\\#"), "{written}");
        assert!(written.contains(&format!("{NO_BREAK_SPACE}{NO_BREAK_SPACE}{NO_BREAK_SPACE}{NO_BREAK_SPACE}indented")), "{written}");
        assert!(written.contains(&format!("\n{NO_BREAK_SPACE}\n")), "an empty line stays: {written}");
    }

    #[test]
    fn styles_that_overlap_and_touch_are_read_back() {
        // "ab" bold, "bc" italic: a / b / c runs, then a link over the whole.
        let formatted = text(
            "abc",
            &[(0, 2, TextEntityType::Bold), (1, 2, TextEntityType::Italic), (0, 3, TextEntityType::TextUrl { url: "https://example.org/a b".into() })],
        );
        assert!(styled_text(&formatted).is_some());
        assert!(markdown(&formatted).unwrap().contains("(<https://example.org/a%20b>)"));
        for kind in [TextEntityType::Underline, TextEntityType::Strikethrough, TextEntityType::Code, TextEntityType::BlockQuote] {
            let formatted = text("one two\nthree", &[(0, 3, TextEntityType::Bold), (2, 11, kind)]);
            assert!(styled_text(&formatted).is_some(), "{:?}", markdown(&formatted));
        }
    }

    #[test]
    fn every_style_takes_effect() {
        // Styled runs next to words and punctuation, touching each other, across a link.
        let formatted = text(
            "x\"bold\"y italic~ strike under code link!",
            &[
                (1, 6, TextEntityType::Bold),
                (7, 1, TextEntityType::Bold),
                (8, 8, TextEntityType::Italic),
                (17, 6, TextEntityType::Strikethrough),
                (24, 5, TextEntityType::Underline),
                (30, 4, TextEntityType::Code),
                (35, 5, TextEntityType::TextUrl { url: "https://example.org".into() }),
            ],
        );
        let styled = format!("{:?}", styled_text(&formatted).expect("Slint reads it"));
        for style in ["Strong", "Emphasis", "Strikethrough", "Underline", "Code", "Link"] {
            assert!(styled.contains(style), "{style} missing: {styled}");
        }
        // (Debug shows a quote as \" and a backslash as \\.)
        for leftover in ["**", "~~", "<u>", "`", "](", "\\\\"] {
            assert!(!styled.contains(leftover), "{leftover:?} left in the text: {styled}");
        }
    }

    #[test]
    fn code_keeps_its_backticks() {
        assert_eq!(code_span(&"a`b".chars().collect::<Vec<_>>()), "``a`b``");
        assert_eq!(code_span(&"`x".chars().collect::<Vec<_>>()), "`` `x ``");
        let formatted = text("run `ls` now", &[(4, 4, TextEntityType::Code)]);
        assert!(styled_text(&formatted).is_some());
    }

    #[test]
    fn a_quote_is_marked_on_each_line() {
        let written = markdown(&text("one\ntwo", &[(0, 7, TextEntityType::BlockQuote)])).unwrap();
        assert!(written.starts_with("│ ") && written.contains("\n│ "), "{written}");
    }
}
