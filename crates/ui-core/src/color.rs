/// An 8-bit sRGB color, useful at asset/API boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Srgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Linear-light, straight-alpha RGBA color.
///
/// Display-list colors stay straight-alpha and linear until the renderer
/// converts them to premultiplied vertex data. This keeps the color contract
/// independent of a particular graphics API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// A UI color expressed as linear-light, straight-alpha RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(LinearRgba);

impl Color {
    pub const TRANSPARENT: Self = Self(LinearRgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    });
    pub const WHITE: Self = Self(LinearRgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    });
    pub const BLACK: Self = Self(LinearRgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    });

    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self(LinearRgba { r, g, b, a })
    }

    pub fn from_srgba8(color: Srgb8) -> Self {
        Self::linear(
            srgb_to_linear(color.r as f32 / 255.0),
            srgb_to_linear(color.g as f32 / 255.0),
            srgb_to_linear(color.b as f32 / 255.0),
            color.a as f32 / 255.0,
        )
    }

    pub const fn from_linear(color: LinearRgba) -> Self {
        Self(color)
    }

    pub const fn linear_rgba(self) -> LinearRgba {
        self.0
    }

    /// Returns linear RGBA suitable for a premultiplied-alpha blend pipeline.
    pub fn premultiplied_linear(self) -> [f32; 4] {
        let c = self.0;
        [c.r * c.a, c.g * c.a, c.b * c.a, c.a]
    }

    pub fn to_srgba8(self) -> Srgb8 {
        let c = self.0;
        Srgb8 {
            r: linear_to_srgb(c.r).mul_add(255.0, 0.5).floor() as u8,
            g: linear_to_srgb(c.g).mul_add(255.0, 0.5).floor() as u8,
            b: linear_to_srgb(c.b).mul_add(255.0, 0.5).floor() as u8,
            a: c.a.clamp(0.0, 1.0).mul_add(255.0, 0.5).floor() as u8,
        }
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgba_round_trip_is_stable_for_representative_values() {
        let source = Srgb8 {
            r: 12,
            g: 128,
            b: 240,
            a: 200,
        };
        assert_eq!(Color::from_srgba8(source).to_srgba8(), source);
    }

    #[test]
    fn premultiplication_happens_once_at_the_renderer_seam() {
        let color = Color::linear(0.8, 0.4, 0.2, 0.5);
        assert_eq!(color.premultiplied_linear(), [0.4, 0.2, 0.1, 0.5]);
    }
}
