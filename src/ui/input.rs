use super::SHORTCUT_CHARS;
use termion::event::{Event, Key};
use unicode_width::UnicodeWidthStr;

pub struct LineEdit {
    data: Vec<char>,
    cursor: usize,
}
pub enum Action {
    Input,
    Completion(usize, String),
    BufChange(i8),
    BufChangeAbs(usize),
    ScrollBack,
    Noop,
    Quit,
}

impl LineEdit {
    pub fn new() -> Self {
        Self {
            data: vec![],
            // Byte count for cursor, not char count.
            cursor: 0,
        }
    }

    pub fn get_string(&self) -> String {
        self.data.iter().collect()
    }

    /// Clear the input.
    pub fn clear(&mut self) {
        self.data.clear();
        self.cursor = 0;
    }

    /// Compute the line-wrapped input and cursor for displaying.
    pub fn get_wrapped(&self, width: u16) -> ((u16, u16), String) {
        const APC: char = '\u{9f}';
        let mut cx = 0;
        let mut cy = 0;
        // To avoid computing cursor through line wrap, we embed a special char
        // and look it up afterwards.
        let wrappable: String = self.data[..self.cursor]
            .iter()
            .chain([APC].iter())
            .chain(self.data[self.cursor..].iter())
            .collect();
        let wrapped = textwrap::wrap_iter(wrappable.as_str(), width as usize)
            .map(|mut l| {
                if cx == 0 {
                    if let Some(pos) = l.find(APC) {
                        cx = l[..pos].width();
                        l = l.replace(APC, "").into();
                    } else {
                        cy += 1;
                    }
                }
                l
            })
            .fold(String::new(), |mut acc, x| {
                if !acc.is_empty() {
                    acc.push('\n');
                }
                acc.push_str(&x[..]);
                acc
            });
        log::trace!("input: {}, {}, {:?}", cx, cy, wrapped);
        ((cx as u16, cy), wrapped)
    }

    pub fn handle_input(&mut self, input: String) -> Action {
        // log::info!("{:?}", s);
        let mut iter = input.bytes().map(|b| Ok(b));
        while let Some(Ok(b)) = iter.next() {
            let event = termion::event::parse_event(b, &mut iter);
            match event {
                Ok(Event::Key(k)) => match k {
                    Key::Char('\n') => return Action::Input,
                    Key::Alt('\r') => {
                        self.data.insert(self.cursor, '\n');
                        self.cursor += 1;
                    }
                    Key::Char('\t') => {
                        return Action::Completion(
                            self.cursor,
                            self.get_string().replace('\n', "."), // escape endlines
                        );
                    }
                    Key::Char(c) => {
                        self.data.insert(self.cursor, c);
                        self.cursor += 1;
                    }
                    Key::Ctrl('c') => return Action::Quit,
                    Key::Ctrl('u') => self.clear(),
                    Key::Up => return Action::ScrollBack,
                    Key::Left => self.cursor = self.cursor.saturating_sub(1),
                    Key::Right => self.cursor = usize::min(self.data.len(), self.cursor + 1),
                    Key::Ctrl('p') => return Action::BufChange(-1),
                    Key::Ctrl('n') => return Action::BufChange(1),
                    Key::Backspace => {
                        if self.cursor != 0 {
                            self.cursor -= 1;
                            self.data.remove(self.cursor);
                        }
                    }
                    Key::Alt(c) => {
                        if let Some(pos) = SHORTCUT_CHARS.find(c) {
                            return Action::BufChangeAbs(pos);
                        }
                    }
                    _ => {
                        log::trace!("ignored input event {:?}", k);
                    }
                },
                Err(e) => {
                    log::trace!("Input error: {}", e);
                }
                _ => {
                    log::trace!("ignored input event {:?}", event);
                }
            }
        }
        Action::Noop
    }

    /// Receive completion data.
    pub fn complete(&mut self, completion: crate::wee::CompletionData) {
        // TODO have a completion menu or something.
        if completion.list.is_empty() {
            return;
        }
        // XXX work around the issue where positions are swapped with unicode.
        let start_pos = i32::min(completion.pos_start, completion.pos_end) as usize;
        let end_pos = usize::min(self.data.len(), completion.pos_end as usize + 1);
        let replace_range = start_pos..end_pos;
        self.cursor = start_pos as usize + completion.list[0].chars().count();
        self.data.splice(replace_range, completion.list[0].chars());
        if completion.add_space == 1 {
            self.data.insert(self.cursor, ' ');
            self.cursor += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wee::CompletionData;

    fn make_completion(pos_start: i32, pos_end: i32, comp: &str, add_space: u8) -> CompletionData {
        CompletionData {
            context: String::new(),
            base_word: String::new(),
            add_space,
            pos_start,
            pos_end,
            list: vec![String::from(comp)],
        }
    }

    #[test]
    fn test_completion() {
        let mut line = LineEdit::new();
        line.complete(make_completion(0, 0, "", 0));
        assert_eq!("", line.get_string().as_str());
        assert_eq!(0, line.cursor);

        line.complete(make_completion(0, 0, "/he", 0));
        assert_eq!("/he", line.get_string().as_str());
        assert_eq!(3, line.cursor);

        line.complete(make_completion(1, 2, "help", 1));
        assert_eq!("/help ", line.get_string().as_str());
        assert_eq!(6, line.cursor);

        line.handle_input(String::from("x"));
        assert_eq!("/help x", line.get_string().as_str());

        line.clear();
        line.data = vec!['f', 'o', 'o', 'x'];
        line.complete(make_completion(0, 2, "foobar", 1));
        assert_eq!("foobar x", line.get_string().as_str());
        assert_eq!(7, line.cursor);
    }

    #[test]
    fn test_completion_unicode() {
        let mut line = LineEdit::new();
        line.handle_input(String::from("☃ /he"));
        assert_eq!(5, line.cursor);
        line.complete(make_completion(3, 4, "help", 0));
        assert_eq!("☃ /help", line.get_string().as_str());
    }

    #[test]
    fn test_cursor_move() {
        let mut line = LineEdit::new();
        assert_eq!(0, line.cursor);
        line.handle_input(String::from("hello"));
        assert_eq!(5, line.cursor);
        line.handle_input(String::from("\x1b[D"));
        assert_eq!(4, line.cursor);
        line.handle_input(String::from("\x1b[D\x1b[D"));
        assert_eq!(2, line.cursor);
        line.handle_input(String::from("\x1b[C"));
        assert_eq!(3, line.cursor);
    }

    #[test]
    fn test_wrap_input() {
        let scenarios = [
            ("", 0, ((0, 0), "")),
            ("foo", 3, ((3, 0), "foo")),
            ("foo bar", 3, ((3, 0), "foo\nbar")),
            ("foo bar", 7, ((3, 1), "foo\nbar")),
            ("foo  bar", 3, ((3, 0), "foo\nbar")),
            ("foobar", 6, ((2, 1), "foob\nar")),
        ];

        for (input, cursor, expected) in scenarios.iter() {
            let line = LineEdit {
                data: input.chars().collect(),
                cursor: *cursor,
            };
            assert_eq!(
                (expected.0, String::from(expected.1)),
                line.get_wrapped(4),
                "wrapping {:?} with cursor at {}",
                input,
                cursor
            );
        }
    }
}
