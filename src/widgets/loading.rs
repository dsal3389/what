use ratatui::{prelude::*, widgets::LineGauge};

pub enum LoadingLineState {
    Loading,
    Failed,
    Success,
}

pub struct LoadingLine<T> {
    title: T,
    state: LoadingLineState,
}

impl<T> LoadingLine<T> {
    pub fn new(title: T, state: LoadingLineState) -> LoadingLine<T> {
        LoadingLine { title, state }
    }
}

impl<'a, T> Widget for LoadingLine<T>
where
    T: Into<Line<'a>>,
{
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let style = match self.state {
            LoadingLineState::Failed => Style::default().red(),
            LoadingLineState::Loading => Style::default().light_blue(),
            LoadingLineState::Success => Style::default().light_green(),
        };
        LineGauge::default()
            .label(self.title)
            .style(style)
            .render(area, buf);
    }
}
