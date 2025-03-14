use std::iter::repeat_n;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{prelude::*, text::ToLine};

pub enum InputType<'a> {
    // will add the given prefixed string to the
    // inputted value
    WithPrefix(&'a str),

    // secure will replace the text with `*` chars
    // after user press enter
    Secure,

    // will keep the text clear in terminal history
    Clear,
}

pub struct InputLine<'a, 'b> {
    placeholder: &'a str,
    content: &'b str,
}

impl<'a, 'b> InputLine<'a, 'b> {
    pub fn new(placeholder: &'a str, content: &'b str) -> InputLine<'a, 'b> {
        InputLine {
            placeholder,
            content,
        }
    }

    /// associated function that reads single line from
    /// input and returns the input buffer, if `None` is returned, it means
    /// the user pressed ctrl + c or esc
    pub fn readline(
        terminal: &mut Terminal<impl Backend>,
        placeholder: &'a str,
        input_type: InputType,
    ) -> Result<Option<String>> {
        let frame_area = terminal.get_frame().area();
        let mut buffer = String::with_capacity(frame_area.width as usize);
        let mut cursor_pos = 0_u16;

        terminal.draw(|frame| {
            frame.render_widget(InputLine::new(placeholder, &buffer), frame.area());
            frame.set_cursor_position((0, frame.area().y));
        })?;

        loop {
            if let Event::Key(key) = event::read()? {
                match key.kind {
                    KeyEventKind::Release => match key.code {
                        _ => {}
                    },
                    KeyEventKind::Press => match key.code {
                        KeyCode::Esc => {
                            terminal.insert_before(1, |buf| {
                                "// esc"
                                    .to_line()
                                    .style(Style::default().dark_gray().italic())
                                    .render(buf.area, buf);
                            })?;
                            break Ok(None);
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            terminal.insert_before(1, |buf| {
                                " // CTRL + C"
                                    .to_line()
                                    .style(Style::default().dark_gray().italic())
                                    .render(buf.area, buf);
                            })?;
                            break Ok(None);
                        }
                        KeyCode::Char(c) => {
                            buffer.insert(cursor_pos as usize, c);
                            cursor_pos += 1;
                        }
                        KeyCode::Right if cursor_pos < buffer.len() as u16 => {
                            cursor_pos += 1;
                        }
                        KeyCode::Left => {
                            cursor_pos = cursor_pos.saturating_sub(1);
                        }
                        KeyCode::Backspace if !buffer.is_empty() => {
                            cursor_pos = cursor_pos.saturating_sub(1);
                            buffer.remove(cursor_pos as usize);
                        }
                        KeyCode::Enter if !buffer.is_empty() => {
                            terminal.insert_before(1, |buf| match input_type {
                                InputType::Clear => buffer.to_line().render(buf.area, buf),
                                InputType::Secure => repeat_n('*', buffer.len())
                                    .collect::<String>()
                                    .to_line()
                                    .render(buf.area, buf),
                                InputType::WithPrefix(prefix) => format!("{} {}", prefix, buffer)
                                    .to_line()
                                    .render(buf.area, buf),
                            })?;
                            break Ok(Some(buffer));
                        }
                        _ => continue,
                    },

                    _ => continue,
                };

                terminal.draw(|frame| {
                    frame.render_widget(InputLine::new(placeholder, &buffer), frame.area());
                    frame.set_cursor_position((cursor_pos, frame.area().y));
                })?;
            }
        }
    }
}

impl Widget for InputLine<'_, '_> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let line = if self.content.is_empty() {
            self.placeholder
                .to_line()
                .style(Style::default().dark_gray().italic())
        } else {
            self.content.to_line()
        };
        line.render(area, buf);
    }
}
