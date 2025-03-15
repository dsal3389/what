use ratatui::{prelude::*, widgets::LineGauge};

enum LoadingLineState {
    Loading,
    Failed,
    Success,
}

pub struct LoadingLine<T> {
    title: T,
    state: LoadingLineState,
}

impl<T> LoadingLine<T> {
    pub fn new(title: T) -> LoadingLine<T> {
        LoadingLine {
            title,
            state: LoadingLineState::Loading,
        }
    }

    /// set the loading line state as success
    pub fn sucess(mut self) -> Self {
        self.state = LoadingLineState::Success;
        self
    }

    /// set the loading line state as failed
    #[allow(dead_code)]
    pub fn failed(mut self) -> Self {
        self.state = LoadingLineState::Failed;
        self
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
