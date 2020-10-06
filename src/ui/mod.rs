use crate::wee::Wee;
use input::LineEdit;
use std::cell::RefCell;
use termion::raw::IntoRawMode;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub mod input;

const SHORTCUT_CHARS: &str = "0123456789qwertyuiop";

type Backend = TermionBackend<termion::raw::RawTerminal<std::io::Stdout>>;

const BUFLIST_DEFAULT_STYLE: Style = Style {
    bg: Some(Color::Rgb(60, 60, 60)),
    fg: Some(Color::White),
    add_modifier: Modifier::empty(),
    sub_modifier: Modifier::empty(),
};
const BUFLIST_SELECTED_STYLE: Style = Style {
    bg: Some(Color::Rgb(100, 100, 100)),
    fg: Some(Color::White),
    add_modifier: Modifier::empty(),
    sub_modifier: Modifier::empty(),
};
const TITLE_DEFAULT_STYLE: Style = Style {
    bg: Some(Color::Rgb(30, 30, 30)),
    fg: Some(Color::Rgb(150, 150, 150)),
    add_modifier: Modifier::empty(),
    sub_modifier: Modifier::empty(),
};
const INPUT_DEFAULT_STYLE: Style = Style {
    bg: Some(Color::Rgb(30, 30, 30)),
    fg: Some(Color::White),
    add_modifier: Modifier::empty(),
    sub_modifier: Modifier::empty(),
};

type Tui = tui::Terminal<Backend>;

pub struct Ui {
    tui: RefCell<Tui>,
    pub input: LineEdit,
}

impl Ui {
    pub fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let mut tui = tui::Terminal::new(TermionBackend::new(stdout)).unwrap();
        // clear on start, as other changes are incremental
        tui.clear().unwrap();
        Ui {
            tui: RefCell::new(tui),
            input: LineEdit::new(),
        }
    }

    pub fn draw(&mut self, wee: &Wee) {
        if let Some(comp_data) = wee.consume_completion() {
            self.input.complete(comp_data);
        }
        View::new(wee).render(self.tui.get_mut(), &self.input)
    }
}

struct View<'w> {
    wee: &'w Wee,
}

impl<'w> View<'w> {
    pub fn new(wee: &'w Wee) -> Self {
        Self { wee }
    }

    pub fn render(self, tui: &mut Tui, input: &LineEdit) {
        tui.draw(|f| {
            let input_width = f.size().width.saturating_sub(50); // FIXME calculate from layout
            let ((cursor_x, cursor_y), input_line) = input.get_wrapped(input_width);
            let current_buffer = self.wee.get_current_buffer();

            let title = if let Some(b) = current_buffer {
                textwrap::wrap(
                    match b.title {
                        Some(ref t) => t.trim(),
                        None => b.full_name.as_str(),
                    },
                    f.size().width as usize,
                )
            } else {
                vec![]
            };
            let vlayout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(title.len() as u16), Constraint::Min(1)])
                .split(f.size());
            f.render_widget(
                Paragraph::new(Text::from(title.join("\n").as_str())).style(TITLE_DEFAULT_STYLE),
                vlayout[0],
            );

            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(
                    [
                        Constraint::Length(30),
                        Constraint::Min(20),
                        Constraint::Length(20),
                    ]
                    .as_ref(),
                )
                .split(vlayout[1]);
            let buf_list = List::new(self.render_buflist())
                .highlight_style(BUFLIST_SELECTED_STYLE)
                .block(Block::default().style(BUFLIST_DEFAULT_STYLE));
            let mut buf_list_state = ListState::default();
            buf_list_state.select(self.wee.get_buffers().iter().position(|b| {
                if let Some(current) = current_buffer {
                    current.full_name == b.full_name
                } else {
                    false
                }
            }));
            f.render_stateful_widget(buf_list, layout[0], &mut buf_list_state);
            f.render_widget(
                Paragraph::new("").block(Block::default().title("Nicks").borders(Borders::LEFT)),
                layout[2],
            );

            let center = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(
                        1 + input_line.chars().filter(|c| c == &'\n').count() as u16,
                    ),
                ])
                .split(layout[1]);
            let buffer = self.render_buffer(center[0].width.checked_sub(30));
            let buffer_scroll = buffer
                .len()
                .checked_sub(center[0].height as usize)
                .unwrap_or(0);
            f.render_widget(
                Paragraph::new(buffer).scroll((buffer_scroll as u16, 0)),
                center[0],
            );
            f.render_widget(
                Paragraph::new(input_line).style(INPUT_DEFAULT_STYLE),
                center[1],
            );
            f.set_cursor(cursor_x + center[1].x, cursor_y + center[1].y);
        })
        .unwrap();
        tui.show_cursor().unwrap();
    }

    fn render_buffer(&self, width: Option<u16>) -> Vec<Spans> {
        let mut list: Vec<Spans> = self
            .wee
            .get_lines()
            .iter()
            .map(|line| {
                let secs: i64 = line.date.parse().unwrap();
                let ts = time::OffsetDateTime::from_unix_timestamp(secs);
                let offset = time::UtcOffset::current_local_offset(); // XXX won't match on DST change, but that's ok.
                if let Some(w) = width {
                    textwrap::wrap(line.message.as_str(), (w - 2u16) as usize)
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            Spans::from(vec![
                                if i == 0 {
                                    Span::from(ts.to_offset(offset).format("%H:%M:%S "))
                                } else {
                                    Span::from("          ")
                                },
                                if i == 0 {
                                    let hl = if line.highlight != 0 {
                                        Style::default().fg(Color::Yellow)
                                    } else {
                                        Style::default()
                                    };
                                    Span::styled(
                                        format!(
                                            "{:<20} ⸽ ",
                                            line.prefix.as_ref().unwrap_or(&String::from(""))
                                        ),
                                        hl,
                                    )
                                } else {
                                    Span::from("                    ⸽ ")
                                },
                                Span::from(String::from(m.as_ref())),
                            ])
                        })
                        .collect()
                } else {
                    vec![Spans::from(vec![
                        Span::from(ts.to_offset(offset).format("%H:%M:%S ")),
                        Span::from(format!(
                            "{:<20}",
                            line.prefix.as_ref().unwrap_or(&String::from("          "))
                        )),
                        Span::from(line.message.clone()),
                    ])]
                }
            })
            .flatten()
            .collect();
        if self.wee.is_scrolling {
            list.push(Spans::from(vec![Span::from("  ⬇⬇⬇ Scrolling ⬇⬇⬇")]))
        }
        list
    }

    fn render_buflist(&self) -> Vec<ListItem<'static>> {
        self.wee
            .get_buffers()
            .iter()
            .enumerate()
            .map(|(i, buf)| {
                let name = match buf.short_name {
                    Some(ref s) => s,
                    None => &buf.full_name,
                };
                ListItem::new(Spans::from(vec![
                    Span::from(" "),
                    Span::styled(
                        format!(
                            "{}. ",
                            char::from(*SHORTCUT_CHARS.as_bytes().get(i).unwrap_or(&0x20))
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::from(name.clone()),
                    Span::from(if buf.hotlist.1 > 0 {
                        format!(" ({})", buf.hotlist.1)
                    } else {
                        String::from("")
                    }),
                    Span::styled(
                        if buf.hotlist.2 > 0 {
                            format!(" ({})", buf.hotlist.2)
                        } else {
                            String::from("")
                        },
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        if buf.hotlist.3 > 0 {
                            format!(" ({})", buf.hotlist.3)
                        } else {
                            String::from("")
                        },
                        Style::default().fg(Color::Red),
                    ),
                ]))
            })
            .collect()
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.tui
            .borrow_mut()
            .clear()
            .expect("Clearing screen on shutdown");
    }
}
