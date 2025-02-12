use ratatui::{prelude::*, widgets::Block};

pub struct LineBuffer<'a> {
    lines: &'a [String],
}

impl<'a> LineBuffer<'a> {
    pub fn new(lines: &'a [String]) -> LineBuffer<'a> {
        LineBuffer { lines }
    }
}

impl Widget for LineBuffer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        self.lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let style = if i == 0 {
                    Style::new().dark_gray()
                } else if i == 1 {
                    Style::new().gray()
                } else {
                    Style::new().white()
                };
                Line::from(line.as_str()).style(style)
            })
            .collect::<Text>()
            .render(area, buf);
    }
}
