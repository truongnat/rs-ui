//! Native window lifecycle and HiDPI metrics. No renderer dependency.

use ui_core::{ScaleFactor, Size};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes, WindowId},
};

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub logical_size: Size,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "UI Runtime Foundation".to_owned(),
            logical_size: Size::new(1120.0, 720.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub logical_size: Size,
    pub physical_size: [u32; 2],
    pub scale_factor: ScaleFactor,
}

impl WindowMetrics {
    pub fn from_window(window: &Window) -> Self {
        let scale_factor = ScaleFactor::new(window.scale_factor() as f32);
        let physical_size = window.inner_size();
        let logical_size = Size::new(
            physical_size.width as f32 / scale_factor.get(),
            physical_size.height as f32 / scale_factor.get(),
        );
        Self {
            logical_size,
            physical_size: [physical_size.width, physical_size.height],
            scale_factor,
        }
    }
}

pub struct UiWindow {
    window: std::sync::Arc<Window>,
    metrics: WindowMetrics,
}

impl UiWindow {
    pub fn open(
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Result<Self, winit::error::OsError> {
        let attributes = WindowAttributes::default()
            .with_title(config.title.clone())
            .with_inner_size(LogicalSize::new(
                config.logical_size.width,
                config.logical_size.height,
            ));
        let window = std::sync::Arc::new(event_loop.create_window(attributes)?);
        let metrics = WindowMetrics::from_window(&window);
        Ok(Self { window, metrics })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }
    pub fn window(&self) -> &std::sync::Arc<Window> {
        &self.window
    }
    pub fn metrics(&self) -> WindowMetrics {
        self.metrics
    }
    pub fn refresh_metrics(&mut self) {
        self.metrics = WindowMetrics::from_window(&self.window);
    }
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            self.metrics.physical_size = [size.width, size.height];
            self.metrics.logical_size = Size::new(
                size.width as f32 / self.metrics.scale_factor.get(),
                size.height as f32 / self.metrics.scale_factor.get(),
            );
        }
    }
}
