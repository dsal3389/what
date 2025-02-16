mod loading;
pub use loading::LoadingLine;
use ratatui::layout::Rect;

pub fn stick_bottom(area: Rect, height: u16) -> Rect {
    Rect::new(area.x, area.bottom() - height, area.width, height)
}
