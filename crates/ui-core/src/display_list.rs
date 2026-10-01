use crate::{Color, Point, Radius, Rect, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextRunId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayCommand {
    FillRect {
        rect: Rect,
        color: Color,
    },
    FillRoundedRect {
        rect: Rect,
        radius: Radius,
        color: Color,
    },
    StrokeRoundedRect {
        rect: Rect,
        radius: Radius,
        stroke: Stroke,
    },
    Line {
        from: Point,
        to: Point,
        stroke: Stroke,
    },
    Image {
        rect: Rect,
        image: ImageId,
        tint: Color,
    },
    Text {
        run: TextRunId,
        origin: Point,
    },
    PushClip(Rect),
    PopClip,
    PushTransform(Transform),
    PopTransform,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    commands: Vec<DisplayCommand>,
}

impl DisplayList {
    pub fn commands(&self) -> &[DisplayCommand] {
        &self.commands
    }
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayListBuilder {
    commands: Vec<DisplayCommand>,
}

impl DisplayListBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) -> &mut Self {
        self.commands.push(DisplayCommand::FillRect { rect, color });
        self
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: Radius, color: Color) -> &mut Self {
        self.commands.push(DisplayCommand::FillRoundedRect {
            rect,
            radius,
            color,
        });
        self
    }

    pub fn stroke_rounded_rect(&mut self, rect: Rect, radius: Radius, stroke: Stroke) -> &mut Self {
        self.commands.push(DisplayCommand::StrokeRoundedRect {
            rect,
            radius,
            stroke,
        });
        self
    }

    pub fn line(&mut self, from: Point, to: Point, stroke: Stroke) -> &mut Self {
        self.commands
            .push(DisplayCommand::Line { from, to, stroke });
        self
    }

    pub fn image(&mut self, rect: Rect, image: ImageId, tint: Color) -> &mut Self {
        self.commands
            .push(DisplayCommand::Image { rect, image, tint });
        self
    }

    pub fn text(&mut self, run: TextRunId, origin: Point) -> &mut Self {
        self.commands.push(DisplayCommand::Text { run, origin });
        self
    }

    pub fn push_clip(&mut self, rect: Rect) -> &mut Self {
        self.commands.push(DisplayCommand::PushClip(rect));
        self
    }

    pub fn pop_clip(&mut self) -> &mut Self {
        self.commands.push(DisplayCommand::PopClip);
        self
    }

    pub fn push_transform(&mut self, transform: Transform) -> &mut Self {
        self.commands.push(DisplayCommand::PushTransform(transform));
        self
    }

    pub fn pop_transform(&mut self) -> &mut Self {
        self.commands.push(DisplayCommand::PopTransform);
        self
    }

    pub fn build(self) -> DisplayList {
        DisplayList {
            commands: self.commands,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_preserves_order_and_keeps_renderer_free() {
        let mut builder = DisplayListBuilder::new();
        builder
            .fill_rect(Rect::ZERO, Color::WHITE)
            .push_clip(Rect::ZERO)
            .pop_clip();
        let list = builder.build();
        assert_eq!(list.commands().len(), 3);
        assert!(matches!(list.commands()[1], DisplayCommand::PushClip(_)));
    }

    #[test]
    fn scoped_state_is_explicit_and_paint_order_is_stable() {
        let mut builder = DisplayListBuilder::new();
        builder
            .push_clip(Rect::ZERO)
            .push_transform(Transform::translation(2.0, 3.0))
            .fill_rect(Rect::ZERO, Color::WHITE)
            .pop_transform()
            .fill_rect(Rect::ZERO, Color::BLACK)
            .pop_clip();
        let commands = builder.build();
        assert!(matches!(
            commands.commands()[0],
            DisplayCommand::PushClip(_)
        ));
        assert!(matches!(
            commands.commands()[1],
            DisplayCommand::PushTransform(_)
        ));
        assert!(matches!(
            commands.commands()[2],
            DisplayCommand::FillRect { .. }
        ));
        assert!(matches!(
            commands.commands()[4],
            DisplayCommand::FillRect { .. }
        ));
        assert!(matches!(commands.commands()[5], DisplayCommand::PopClip));
    }
}
