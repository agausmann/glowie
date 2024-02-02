use crate::GraphicsContext;

pub struct Scope {}

impl Scope {
    pub fn new(gfx: GraphicsContext) -> Self {
        Self {}
    }

    pub fn draw(&mut self, frame_view: &wgpu::TextureView, encoder: &mut wgpu::CommandEncoder) {}

    pub fn window_resized(&mut self) {}
}
