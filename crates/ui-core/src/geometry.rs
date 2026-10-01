use std::ops::{Add, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub const ZERO: Self = Self {
        min: Point::ZERO,
        max: Point::ZERO,
    };

    pub const fn from_min_max(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    pub const fn from_min_size(min: Point, size: Size) -> Self {
        Self {
            min,
            max: Point::new(min.x + size.width, min.y + size.height),
        }
    }

    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }
    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }
    pub fn size(self) -> Size {
        Size::new(self.width(), self.height())
    }
    pub fn center(self) -> Point {
        Point::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    pub fn inset(self, amount: f32) -> Self {
        Self::from_min_max(
            Point::new(self.min.x + amount, self.min.y + amount),
            Point::new(self.max.x - amount, self.max.y - amount),
        )
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let min = Point::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y));
        let max = Point::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y));
        (max.x >= min.x && max.y >= min.y).then_some(Self::from_min_max(min, max))
    }

    pub fn snap_to_physical(self, scale_factor: ScaleFactor) -> Self {
        let scale = scale_factor.get();
        Self::from_min_max(
            Point::new(
                (self.min.x * scale).round() / scale,
                (self.min.y * scale).round() / scale,
            ),
            Point::new(
                (self.max.x * scale).round() / scale,
                (self.max.y * scale).round() / scale,
            ),
        )
    }

    pub fn to_physical(self, scale_factor: ScaleFactor) -> PhysicalRect {
        let scale = scale_factor.get();
        PhysicalRect {
            min: [
                ((self.min.x * scale).round().max(0.0)) as u32,
                ((self.min.y * scale).round().max(0.0)) as u32,
            ],
            max: [
                ((self.max.x * scale).round().max(0.0)) as u32,
                ((self.max.y * scale).round().max(0.0)) as u32,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalRect {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

impl PhysicalRect {
    pub fn width(self) -> u32 {
        self.max[0].saturating_sub(self.min[0])
    }
    pub fn height(self) -> u32 {
        self.max[1].saturating_sub(self.min[1])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleFactor(f32);

impl ScaleFactor {
    pub fn new(value: f32) -> Self {
        Self(value.max(0.01))
    }
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for ScaleFactor {
    fn default() -> Self {
        Self(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Radius {
    pub const ZERO: Self = Self {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };
    pub const fn all(value: f32) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
        }
    }

    pub fn clamp_to(self, rect: Rect) -> Self {
        let limit = (rect.width().min(rect.height()) * 0.5).max(0.0);
        Self {
            top_left: self.top_left.clamp(0.0, limit),
            top_right: self.top_right.clamp(0.0, limit),
            bottom_right: self.bottom_right.clamp(0.0, limit),
            bottom_left: self.bottom_left.clamp(0.0, limit),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: crate::Color,
}

impl Stroke {
    pub const fn new(width: f32, color: crate::Color) -> Self {
        Self { width, color }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub matrix: [[f32; 3]; 2],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    };

    pub const fn translation(x: f32, y: f32) -> Self {
        Self {
            matrix: [[1.0, 0.0, x], [0.0, 1.0, y]],
        }
    }
    pub const fn scale(x: f32, y: f32) -> Self {
        Self {
            matrix: [[x, 0.0, 0.0], [0.0, y, 0.0]],
        }
    }

    pub fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self {
            matrix: [[cos, -sin, 0.0], [sin, cos, 0.0]],
        }
    }

    pub const fn shear(x_by_y: f32, y_by_x: f32) -> Self {
        Self {
            matrix: [[1.0, x_by_y, 0.0], [y_by_x, 1.0, 0.0]],
        }
    }

    pub fn is_axis_aligned(self) -> bool {
        self.matrix[0][1].abs() < f32::EPSILON && self.matrix[1][0].abs() < f32::EPSILON
    }

    pub fn transform_point(self, point: Point) -> Point {
        Point::new(
            self.matrix[0][0] * point.x + self.matrix[0][1] * point.y + self.matrix[0][2],
            self.matrix[1][0] * point.x + self.matrix[1][1] * point.y + self.matrix[1][2],
        )
    }

    pub fn inverse(self) -> Option<Self> {
        let a = self.matrix;
        let determinant = a[0][0] * a[1][1] - a[0][1] * a[1][0];
        if determinant.abs() <= f32::EPSILON {
            return None;
        }
        let inverse_determinant = 1.0 / determinant;
        Some(Self {
            matrix: [
                [
                    a[1][1] * inverse_determinant,
                    -a[0][1] * inverse_determinant,
                    (a[0][1] * a[1][2] - a[1][1] * a[0][2]) * inverse_determinant,
                ],
                [
                    -a[1][0] * inverse_determinant,
                    a[0][0] * inverse_determinant,
                    (a[1][0] * a[0][2] - a[0][0] * a[1][2]) * inverse_determinant,
                ],
            ],
        })
    }

    /// Compose transforms in application order: `self`, then `next`.
    /// A child-local transform composed with its parent is `child.then(parent)`.
    pub fn then(self, next: Self) -> Self {
        let a = self.matrix;
        let b = next.matrix;
        Self {
            matrix: [
                [
                    b[0][0] * a[0][0] + b[0][1] * a[1][0],
                    b[0][0] * a[0][1] + b[0][1] * a[1][1],
                    b[0][0] * a[0][2] + b[0][1] * a[1][2] + b[0][2],
                ],
                [
                    b[1][0] * a[0][0] + b[1][1] * a[1][0],
                    b[1][0] * a[0][1] + b[1][1] * a[1][1],
                    b[1][0] * a[0][2] + b[1][1] * a[1][2] + b[1][2],
                ],
            ],
        }
    }

    pub fn transform_rect(self, rect: Rect) -> Rect {
        let points = [
            rect.min,
            Point::new(rect.max.x, rect.min.y),
            rect.max,
            Point::new(rect.min.x, rect.max.y),
        ]
        .map(|point| self.transform_point(point));
        let mut min = points[0];
        let mut max = points[0];
        for point in points.into_iter().skip(1) {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        Rect::from_min_max(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_uses_physical_boundaries() {
        let rect = Rect::from_min_max(Point::new(0.25, 1.1), Point::new(10.24, 5.74));
        let snapped = rect.snap_to_physical(ScaleFactor::new(2.0));
        assert_eq!(
            snapped,
            Rect::from_min_max(Point::new(0.5, 1.0), Point::new(10.0, 5.5))
        );
    }

    #[test]
    fn physical_rect_is_integer_and_scale_aware() {
        let rect = Rect::from_min_size(Point::new(3.0, 4.0), Size::new(10.0, 5.0));
        assert_eq!(
            rect.to_physical(ScaleFactor::new(2.0)),
            PhysicalRect {
                min: [6, 8],
                max: [26, 18]
            }
        );
    }

    #[test]
    fn nested_transforms_apply_local_then_parent() {
        let parent = Transform::translation(10.0, 5.0);
        let child = Transform::scale(2.0, 3.0);
        let world = child.then(parent);
        assert_eq!(
            world.transform_point(Point::new(4.0, 2.0)),
            Point::new(18.0, 11.0)
        );
    }

    #[test]
    fn affine_inverse_round_trips_points_and_rejects_singular_matrices() {
        let transform = Transform::translation(10.0, 5.0).then(Transform::rotation(0.25));
        let point = Point::new(4.0, -2.0);
        let transformed = transform.transform_point(point);
        let restored = transform.inverse().unwrap().transform_point(transformed);
        assert!((restored.x - point.x).abs() < 0.0001);
        assert!((restored.y - point.y).abs() < 0.0001);
        assert!(Transform::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn rotation_and_shear_are_affine_and_detected_as_non_axis_aligned() {
        let rotation = Transform::rotation(std::f32::consts::FRAC_PI_2);
        let rotated = rotation.transform_point(Point::new(2.0, 0.0));
        assert!(rotated.x.abs() < 0.0001);
        assert!((rotated.y - 2.0).abs() < 0.0001);
        assert!(!rotation.is_axis_aligned());
        assert!(!Transform::shear(0.5, 0.0).is_axis_aligned());
    }
}
