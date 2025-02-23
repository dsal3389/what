use ratatui::{prelude::*, text::ToLine};

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
