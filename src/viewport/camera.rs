//! Orbit camera for the model viewport. Pure maths, no browser types.
//!
//! Z is up (the printer's convention). Matrices are column-major, ready for
//! `uniformMatrix4fv`.

/// Vertical field of view, 45 degrees.
const FOV: f32 = std::f32::consts::FRAC_PI_4;
/// Fraction of the view the fitted model's bounding sphere fills.
const FILL: f32 = 0.8;
/// Pitch limit, kept short of straight up or down so `look_at` stays stable.
const MAX_PITCH_DEG: f32 = 89.0;
/// Smallest `w` a point may have and still be projected.
const MIN_W: f32 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: [f32; 3],
    /// Radius of the model's bounding sphere; the zoom limits scale with it.
    pub radius: f32,
}

impl Orbit {
    /// Frame the box `min..max`.
    ///
    /// The bounding sphere fills 80% of the view vertically. `fit` does not
    /// know the canvas shape, so horizontally the model fits for any aspect
    /// (width / height) of about 0.78 or wider. The viewport is always 4:3, so
    /// this holds; a narrower canvas would clip the sides.
    ///
    /// Degenerate boxes (a point, a line) still give a finite camera.
    /// Non-finite bounds give a default orbit around a 256 mm cube.
    pub fn fit(min: [f32; 3], max: [f32; 3]) -> Orbit {
        if !min.iter().chain(max.iter()).all(|v| v.is_finite()) {
            return Orbit::fit([0.0; 3], [256.0; 3]);
        }
        let target = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let dx = max[0] - min[0];
        let dy = max[1] - min[1];
        let dz = max[2] - min[2];
        let radius = ((dx * dx + dy * dy + dz * dz).sqrt() * 0.5).max(0.001);
        Orbit {
            yaw: (-45f32).to_radians(),
            pitch: 30f32.to_radians(),
            distance: radius / (FOV * 0.5).sin() / FILL,
            target,
            radius,
        }
    }

    pub fn eye(&self) -> [f32; 3] {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        [
            self.target[0] + self.distance * cp * cy,
            self.target[1] + self.distance * cp * sy,
            self.target[2] + self.distance * sp,
        ]
    }

    /// Non-finite deltas are ignored, so one bad input cannot poison the state.
    pub fn rotate(&mut self, dyaw: f32, dpitch: f32) {
        let limit = MAX_PITCH_DEG.to_radians();
        if dyaw.is_finite() {
            self.yaw += dyaw;
        }
        if dpitch.is_finite() {
            self.pitch = (self.pitch + dpitch).clamp(-limit, limit);
        }
    }

    /// A non-finite factor is ignored.
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() {
            return;
        }
        self.distance = (self.distance * factor).clamp(self.radius * 0.5, self.radius * 20.0);
    }

    /// Projection times view, column-major.
    pub fn view_proj(&self, aspect: f32) -> [f32; 16] {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let near = self.distance * 0.01;
        let far = self.distance + 4.0 * self.radius;
        let proj = perspective(FOV, aspect, near, far);
        let view = look_at(self.eye(), self.target, [0.0, 0.0, 1.0]);
        mul(&proj, &view)
    }

    /// Screen position in pixels, or `None` when the point is behind the
    /// camera (or so close to its plane that the divide would blow up).
    ///
    /// The near and far planes are not tested: a point in front of the camera
    /// still projects even when GL would clip it, which is what pins want.
    pub fn project(&self, p: [f32; 3], width: f32, height: f32) -> Option<(f32, f32)> {
        let vp = self.view_proj(width / height);
        let at =
            |row: usize| vp[row] * p[0] + vp[4 + row] * p[1] + vp[8 + row] * p[2] + vp[12 + row];
        let (x, y, w) = (at(0), at(1), at(3));
        if w.is_nan() || w <= MIN_W {
            return None;
        }
        let nx = x / w;
        let ny = y / w;
        let (sx, sy) = ((nx * 0.5 + 0.5) * width, (1.0 - (ny * 0.5 + 0.5)) * height);
        (sx.is_finite() && sy.is_finite()).then_some((sx, sy))
    }
}

fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fov * 0.5).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) / (near - far);
    m[11] = -1.0;
    m[14] = 2.0 * far * near / (near - far);
    m
}

fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        s[0],
        u[0],
        -f[0],
        0.0,
        s[1],
        u[1],
        -f[1],
        0.0,
        s[2],
        u[2],
        -f[2],
        0.0,
        -dot(s, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

/// `a * b` for column-major 4x4 matrices.
fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = dot(a, a).sqrt();
    if len > 0.0 {
        [a[0] / len, a[1] / len, a[2] / len]
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_centres_the_model() {
        let o = Orbit::fit([0., 0., 0.], [10., 10., 10.]);
        assert_eq!(o.target, [5., 5., 5.]);
        let (x, y) = o.project([5., 5., 5.], 400.0, 300.0).unwrap();
        assert!((x - 200.0).abs() < 0.5 && (y - 150.0).abs() < 0.5);
    }

    #[test]
    fn the_whole_model_is_in_view_after_fit() {
        let o = Orbit::fit([0., 0., 0.], [10., 20., 5.]);
        for corner in [[0., 0., 0.], [10., 20., 5.], [10., 0., 5.], [0., 20., 0.]] {
            let (x, y) = o.project(corner, 400.0, 300.0).unwrap();
            assert!(
                (0.0..=400.0).contains(&x) && (0.0..=300.0).contains(&y),
                "{corner:?} -> {x},{y}"
            );
        }
    }

    #[test]
    fn pitch_is_clamped_and_zoom_is_bounded() {
        let mut o = Orbit::fit([0., 0., 0.], [1., 1., 1.]);
        o.rotate(0.0, 10.0);
        assert!(o.pitch <= 89f32.to_radians() + 1e-6);
        let d = o.distance;
        o.zoom(1e-6);
        assert!(o.distance >= o.radius * 0.5 - 1e-6 && o.distance < d);
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        let o = Orbit::fit([0., 0., 0.], [1., 1., 1.]);
        let eye = o.eye();
        let behind = [
            eye[0] * 3.0 - o.target[0] * 2.0,
            eye[1] * 3.0 - o.target[1] * 2.0,
            eye[2] * 3.0 - o.target[2] * 2.0,
        ];
        assert_eq!(o.project(behind, 400.0, 300.0), None);
    }

    #[test]
    fn a_zero_size_model_stays_finite() {
        let o = Orbit::fit([3., 4., 5.], [3., 4., 5.]);
        assert!(o.radius > 0.0 && o.distance.is_finite() && o.distance > 0.0);
        assert!(o.eye().iter().all(|v| v.is_finite()));
        assert!(o.view_proj(1.5).iter().all(|v| v.is_finite()));
        let (x, y) = o.project([3., 4., 5.], 400.0, 300.0).unwrap();
        assert!(x.is_finite() && y.is_finite());
        assert!((x - 200.0).abs() < 0.5 && (y - 150.0).abs() < 0.5);
    }

    const ASPECT_W: f32 = 400.0;
    const ASPECT_H: f32 = 300.0;

    fn unit_cube() -> Orbit {
        Orbit::fit([0., 0., 0.], [10., 10., 10.])
    }

    #[test]
    fn a_point_above_the_target_projects_above_the_centre() {
        let o = unit_cube();
        let (_, cy) = o.project(o.target, ASPECT_W, ASPECT_H).unwrap();
        let (_, y) = o.project([5., 5., 7.], ASPECT_W, ASPECT_H).unwrap();
        assert!(y < cy - 1.0, "above should be a smaller y: {y} vs {cy}");
    }

    #[test]
    fn right_and_up_are_not_mirrored() {
        // Looking along -x from +x with Z up, +y is on the right.
        let mut o = unit_cube();
        o.yaw = 0.0;
        o.pitch = 0.0;
        let (cx, cy) = o.project(o.target, ASPECT_W, ASPECT_H).unwrap();
        let (rx, _) = o.project([5., 7., 5.], ASPECT_W, ASPECT_H).unwrap();
        let (_, uy) = o.project([5., 5., 7.], ASPECT_W, ASPECT_H).unwrap();
        assert!(rx > cx + 1.0, "+y should be right of centre: {rx} vs {cx}");
        assert!(uy < cy - 1.0, "+z should be above centre: {uy} vs {cy}");
    }

    #[test]
    fn the_fitted_sphere_fills_about_eighty_percent_of_the_height() {
        let mut o = unit_cube();
        o.yaw = 0.0;
        o.pitch = 0.0;
        let r = o.radius;
        let top = [o.target[0], o.target[1], o.target[2] + r];
        let bottom = [o.target[0], o.target[1], o.target[2] - r];
        let (_, y0) = o.project(top, ASPECT_W, ASPECT_H).unwrap();
        let (_, y1) = o.project(bottom, ASPECT_W, ASPECT_H).unwrap();
        // The nearest points of the sphere's poles project a little inside
        // the true silhouette, so a bit under 0.8.
        let fill = (y1 - y0) / ASPECT_H;
        assert!((0.72..=0.80).contains(&fill), "fill {fill}");
    }

    #[test]
    fn the_aspect_ratio_keeps_pixels_square() {
        let mut o = unit_cube();
        o.yaw = 0.0;
        o.pitch = 0.0;
        let (cx, cy) = o.project(o.target, ASPECT_W, ASPECT_H).unwrap();
        let (rx, _) = o.project([5., 7., 5.], ASPECT_W, ASPECT_H).unwrap();
        let (_, uy) = o.project([5., 5., 7.], ASPECT_W, ASPECT_H).unwrap();
        // Equal world offsets right and up cover equal numbers of pixels.
        assert!(
            ((rx - cx) - (cy - uy)).abs() < 0.5,
            "{} vs {}",
            rx - cx,
            cy - uy
        );
    }

    #[test]
    fn a_wide_model_fits_the_viewport_after_fit() {
        let o = Orbit::fit([0., 0., 0.], [100., 10., 10.]);
        for x in [0., 100.] {
            for y in [0., 10.] {
                for z in [0., 10.] {
                    let (px, py) = o.project([x, y, z], ASPECT_W, ASPECT_H).unwrap();
                    assert!(
                        (0.0..=ASPECT_W).contains(&px) && (0.0..=ASPECT_H).contains(&py),
                        "{x},{y},{z} -> {px},{py}"
                    );
                }
            }
        }
    }

    #[test]
    fn non_finite_input_cannot_poison_the_camera() {
        let mut o = unit_cube();
        let before = o;
        o.rotate(f32::NAN, f32::NAN);
        o.rotate(f32::INFINITY, f32::NEG_INFINITY);
        o.zoom(f32::NAN);
        o.zoom(f32::INFINITY);
        assert_eq!(o, before);
        // A finite half still applies.
        o.rotate(f32::NAN, 0.1);
        assert_eq!(o.yaw, before.yaw);
        assert!(o.pitch > before.pitch);
    }

    #[test]
    fn non_finite_bounds_give_a_usable_default() {
        for (min, max) in [
            ([f32::NAN; 3], [1.; 3]),
            ([0.; 3], [f32::INFINITY; 3]),
            ([f32::NEG_INFINITY, 0., 0.], [1.; 3]),
        ] {
            let o = Orbit::fit(min, max);
            assert!(o.target.iter().all(|v| v.is_finite()));
            assert!(o.radius.is_finite() && o.distance.is_finite() && o.distance > 0.0);
            let (x, y) = o.project(o.target, ASPECT_W, ASPECT_H).unwrap();
            assert!((x - 200.0).abs() < 0.5 && (y - 150.0).abs() < 0.5);
        }
    }

    #[test]
    fn project_does_not_clip_to_the_near_and_far_planes() {
        let o = unit_cube();
        let eye = o.eye();
        let toward = |t: f32| {
            [
                eye[0] + (o.target[0] - eye[0]) * t,
                eye[1] + (o.target[1] - eye[1]) * t,
                eye[2] + (o.target[2] - eye[2]) * t,
            ]
        };
        // Between the eye and the near plane (near = 0.01 * distance).
        assert!(o.project(toward(0.005), ASPECT_W, ASPECT_H).is_some());
        // Beyond the far plane (far = distance + 4r).
        let far_t = (o.distance + 8.0 * o.radius) / o.distance;
        let (x, y) = o.project(toward(far_t), ASPECT_W, ASPECT_H).unwrap();
        assert!((x - 200.0).abs() < 0.5 && (y - 150.0).abs() < 0.5);
    }
}
