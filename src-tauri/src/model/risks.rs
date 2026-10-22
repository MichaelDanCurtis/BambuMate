//! Geometry risks: places where the model may print badly, found from the
//! mesh alone and shown as numbered pins. They are hints, not verdicts.

use super::mesh::{edge_groups, face_normals, has_normal, sorted_sides, tri_area, Mesh, Vec3};
use serde::Serialize;

/// A face is an overhang when its normal's z is below this: more than 45
/// degrees past vertical, facing down.
// The spec's 0.7071, kept as written rather than 1/sqrt(2).
#[allow(clippy::approx_constant)]
pub const OVERHANG_MAX_NZ: f32 = -0.7071;
/// Faces whose lowest point is within this many mm of the bed are resting on
/// it, not overhanging it, and count towards bed contact.
pub const BED_ZONE_MM: f32 = 0.3;
/// Overhang regions smaller than this many mm² are ignored.
pub const MIN_OVERHANG_AREA_MM2: f64 = 4.0;
/// Most overhang regions reported.
pub const MAX_OVERHANG_REGIONS: usize = 8;
/// A face counts as touching the bed only if it faces nearly straight down.
pub const BED_CONTACT_MAX_NZ: f32 = -0.9;
/// Bed contact under this many mm² is a risk.
pub const MIN_BED_CONTACT_MM2: f64 = 20.0;
/// Bed contact under this fraction of the footprint is a risk.
pub const MIN_BED_CONTACT_FRACTION: f64 = 0.05;
/// Height over the narrower footprint width above this is tall and thin.
pub const TALL_THIN_RATIO: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    Overhang,
    SmallBedContact,
    TallThin,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Risk {
    pub kind: RiskKind,
    pub position: Vec3,
    pub value: f32,
    pub message: String,
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        // Path halving.
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Whether triangle `i` rests on the bed: every one of its vertices is
/// within [`BED_ZONE_MM`] of the model's lowest point `min_z`. A face with
/// even one vertex above that band is rising off the bed: if it faces
/// steeply down it is an overhang, not bed contact, so overhang and contact
/// partition the downward faces.
fn rests_on_bed(mesh: &Mesh, i: usize, min_z: f32) -> bool {
    mesh.triangles[i]
        .iter()
        .all(|&v| mesh.positions[v as usize][2] <= min_z + BED_ZONE_MM)
}

/// Overhang regions (faces joined by shared edges) of at least
/// [`MIN_OVERHANG_AREA_MM2`], largest first, at most [`MAX_OVERHANG_REGIONS`].
/// Each is its area and area-weighted centroid.
fn overhang_regions(mesh: &Mesh, normals: &[Vec3], min_z: f32) -> Vec<(f64, Vec3)> {
    let is_overhang = |i: usize| {
        has_normal(normals[i]) && normals[i][2] < OVERHANG_MAX_NZ && !rests_on_bed(mesh, i, min_z)
    };

    // Only overhang faces are linked, so the edge list stays small.
    let sides = sorted_sides(mesh, is_overhang);
    let mut parent: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
    for group in edge_groups(&sides) {
        let first = group[0].2;
        for &(_, _, f) in &group[1..] {
            let (ra, rb) = (find(&mut parent, first), find(&mut parent, f));
            if ra != rb {
                parent[rb as usize] = ra;
            }
        }
    }

    drop(sides);

    // (region root, face area, area x centroid) per overhang face, grouped by
    // sorting rather than a Vec per region.
    let mut faces: Vec<(u32, f64, [f64; 3])> = Vec::new();
    for f in (0..mesh.triangles.len()).filter(|&f| is_overhang(f)) {
        let [a, b, c] = mesh.tri(f);
        let area = tri_area(a, b, c) as f64;
        let m = [
            (a[0] + b[0] + c[0]) as f64 / 3.0,
            (a[1] + b[1] + c[1]) as f64 / 3.0,
            (a[2] + b[2] + c[2]) as f64 / 3.0,
        ];
        faces.push((
            find(&mut parent, f as u32),
            area,
            [area * m[0], area * m[1], area * m[2]],
        ));
    }
    drop(parent);
    faces.sort_unstable_by_key(|f| f.0);

    let mut regions: Vec<(f64, Vec3)> = Vec::new();
    for run in faces.chunk_by(|x, y| x.0 == y.0) {
        let area: f64 = run.iter().map(|f| f.1).sum();
        if area < MIN_OVERHANG_AREA_MM2 {
            continue;
        }
        let sum = run.iter().fold([0f64; 3], |s, f| {
            [s[0] + f.2[0], s[1] + f.2[1], s[2] + f.2[2]]
        });
        regions.push((
            area,
            [
                (sum[0] / area) as f32,
                (sum[1] / area) as f32,
                (sum[2] / area) as f32,
            ],
        ));
    }
    // Largest first; equal areas fall back to position so the order does not
    // depend on triangle order.
    regions.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1[2].total_cmp(&b.1[2]))
            .then(a.1[0].total_cmp(&b.1[0]))
            .then(a.1[1].total_cmp(&b.1[1]))
    });
    regions.truncate(MAX_OVERHANG_REGIONS);
    regions
}

/// Where the model may print badly, as pins: overhangs first, then bed
/// contact, then tall and thin. An empty mesh has none. The model is taken as
/// oriented in the file.
pub fn find_risks(mesh: &Mesh) -> Vec<Risk> {
    let Some((lo, hi)) = mesh.bounds() else {
        return Vec::new();
    };
    let normals = face_normals(mesh);
    let mut risks = Vec::new();

    for (area, position) in overhang_regions(mesh, &normals, lo[2]) {
        risks.push(Risk {
            kind: RiskKind::Overhang,
            position,
            value: area as f32,
            message: format!("Overhang · {area:.0} mm² steeper than 45° — may need support"),
        });
    }

    // Faces lying on the bed: facing down and resting on it.
    let mut contact = 0f64;
    let mut weighted = [0f64; 3];
    for (i, &n) in normals.iter().enumerate() {
        if !has_normal(n) || n[2] >= BED_CONTACT_MAX_NZ {
            continue;
        }
        if !rests_on_bed(mesh, i, lo[2]) {
            continue;
        }
        let [a, b, c] = mesh.tri(i);
        let area = tri_area(a, b, c) as f64;
        contact += area;
        for k in 0..3 {
            weighted[k] += area * (a[k] + b[k] + c[k]) as f64 / 3.0;
        }
    }
    let footprint = (hi[0] - lo[0]) as f64 * (hi[1] - lo[1]) as f64;
    if contact < MIN_BED_CONTACT_MM2 || contact < MIN_BED_CONTACT_FRACTION * footprint {
        let position = if contact > 0.0 {
            [
                (weighted[0] / contact) as f32,
                (weighted[1] / contact) as f32,
                (weighted[2] / contact) as f32,
            ]
        } else {
            [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, lo[2]]
        };
        risks.push(Risk {
            kind: RiskKind::SmallBedContact,
            position,
            value: contact as f32,
            message: format!(
                "Small bed contact · {contact:.0} mm² — may lift off; consider a brim"
            ),
        });
    }

    let height = hi[2] - lo[2];
    let width = (hi[0] - lo[0]).min(hi[1] - lo[1]);
    if width > 0.0 && height / width > TALL_THIN_RATIO {
        let ratio = height / width;
        risks.push(Risk {
            kind: RiskKind::TallThin,
            position: [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, hi[2]],
            value: ratio,
            message: format!("Tall and thin · {ratio:.0}:1 — may wobble; slow down or add support"),
        });
    }
    risks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mesh::Welder;

    fn boxes(list: &[(Vec3, Vec3)]) -> Mesh {
        let mut w = Welder::default();
        for &(lo, hi) in list {
            let v = |x: usize, y: usize, z: usize| {
                [[lo[0], hi[0]][x], [lo[1], hi[1]][y], [lo[2], hi[2]][z]]
            };
            let (a, b, c, d) = (v(0, 0, 0), v(1, 0, 0), v(1, 1, 0), v(0, 1, 0));
            let (e, f, g, h) = (v(0, 0, 1), v(1, 0, 1), v(1, 1, 1), v(0, 1, 1));
            for t in [
                [a, c, b],
                [a, d, c],
                [e, f, g],
                [e, g, h],
                [a, b, f],
                [a, f, e],
                [b, c, g],
                [b, g, f],
                [c, d, h],
                [c, h, g],
                [d, a, e],
                [d, e, h],
            ] {
                w.push_triangle(t[0], t[1], t[2]);
            }
        }
        w.finish()
    }

    fn of_kind(risks: Vec<Risk>, kind: RiskKind) -> Vec<Risk> {
        risks.into_iter().filter(|r| r.kind == kind).collect()
    }

    /// A cone balanced on its tip: apex at the origin, a ring of radius 5 at
    /// z = 10, and a fan closing the top.
    fn cone_on_its_tip() -> Mesh {
        let segments = 32;
        let ring = |i: usize| {
            let t = std::f32::consts::TAU * (i % segments) as f32 / segments as f32;
            [5.0 * t.cos(), 5.0 * t.sin(), 10.0]
        };
        let mut w = Welder::default();
        for i in 0..segments {
            // Sides face outwards and down; the cap faces up.
            w.push_triangle([0., 0., 0.], ring(i + 1), ring(i));
            w.push_triangle([0., 0., 10.], ring(i), ring(i + 1));
        }
        w.finish()
    }

    #[test]
    fn a_block_on_the_bed_has_no_risks() {
        assert!(find_risks(&boxes(&[([0., 0., 0.], [20., 20., 10.])])).is_empty());
    }

    #[test]
    fn an_empty_mesh_has_no_risks() {
        assert!(find_risks(&Mesh::default()).is_empty());
        let loose = Mesh {
            positions: vec![[0., 0., 0.]],
            triangles: vec![],
        };
        assert!(find_risks(&loose).is_empty());
    }

    #[test]
    fn a_t_shape_overhangs_under_its_bar() {
        // A 4x4x10 post with an arm out of each side at z = 10..12. Each
        // arm's underside, 8 x 4 = 32 mm², is its own overhang region.
        let t = boxes(&[
            ([8., 0., 0.], [12., 4., 12.]),
            ([0., 0., 10.], [8., 4., 12.]),
            ([12., 0., 10.], [20., 4., 12.]),
        ]);
        let over = of_kind(find_risks(&t), RiskKind::Overhang);
        assert_eq!(over.len(), 2, "{over:?}");
        let total: f32 = over.iter().map(|r| r.value).sum();
        assert!((total - 64.0).abs() < 1.0, "{over:?}");
        assert!(over.iter().all(|r| (r.position[2] - 10.0).abs() < 0.01));
        assert!(over[0].message.starts_with("Overhang · "));
    }

    #[test]
    fn a_thin_pillar_is_tall_and_has_small_contact() {
        let kinds: Vec<_> = find_risks(&boxes(&[([0., 0., 0.], [2., 2., 20.])]))
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert!(kinds.contains(&RiskKind::TallThin));
        assert!(kinds.contains(&RiskKind::SmallBedContact));
    }

    #[test]
    fn downward_faces_on_the_bed_are_not_overhangs() {
        let risks = find_risks(&boxes(&[([0., 0., 0.], [30., 30., 5.])]));
        assert!(risks.iter().all(|r| r.kind != RiskKind::Overhang));
    }

    #[test]
    fn messages_hedge() {
        let risks = find_risks(&boxes(&[([0., 0., 0.], [2., 2., 20.])]));
        assert!(!risks.is_empty());
        assert!(risks.iter().all(|r| r.message.contains("may")), "{risks:?}");
    }

    #[test]
    fn a_cone_on_its_tip_has_small_bed_contact() {
        let risks = find_risks(&cone_on_its_tip());
        // The sides lean at about 27° from vertical: n.z is about -0.447,
        // well short of an overhang.
        assert!(of_kind(risks.clone(), RiskKind::Overhang).is_empty());
        let contact = of_kind(risks, RiskKind::SmallBedContact);
        assert_eq!(contact.len(), 1);
        // No contact area, so the pin sits under the model's centre.
        assert_eq!(contact[0].value, 0.0);
        assert!(contact[0].position[0].abs() < 1e-3);
        assert!(contact[0].position[1].abs() < 1e-3);
        assert_eq!(contact[0].position[2], 0.0);
        assert!(contact[0].message.starts_with("Small bed contact · 0 mm²"));
    }

    #[test]
    fn small_overhangs_are_ignored() {
        // A 1.5 x 1.5 mm cap (2.25 mm²) on a thin post: under 4 mm².
        let m = boxes(&[
            ([0., 0., 0.], [1., 1., 10.]),
            ([-0.25, -0.25, 10.], [1.25, 1.25, 11.]),
        ]);
        assert!(of_kind(find_risks(&m), RiskKind::Overhang).is_empty());
    }

    #[test]
    fn at_most_eight_overhangs_are_reported_largest_first() {
        // Ten separate mushrooms; cap i is (3 + i/2) mm square.
        let mut list = Vec::new();
        for i in 0..10 {
            let x = i as f32 * 20.0;
            let s = 3.0 + i as f32 * 0.5;
            list.push(([x, 0., 0.], [x + 1., 1., 10.]));
            list.push(([x - 1., -1., 10.], [x - 1. + s, -1. + s, 11.]));
        }
        let over = of_kind(find_risks(&boxes(&list)), RiskKind::Overhang);
        assert_eq!(over.len(), 8);
        assert!(over.windows(2).all(|w| w[0].value >= w[1].value));
        // Cap 9 is 7.5 mm square.
        assert!((over[0].value - 56.25).abs() < 0.01, "{over:?}");
    }

    #[test]
    fn equal_overhangs_come_out_in_a_fixed_order() {
        let list = [
            ([0., 0., 0.], [1., 1., 10.]),
            ([-1., -1., 10.], [3., 3., 11.]),
            ([20., 0., 0.], [21., 1., 10.]),
            ([19., -1., 10.], [23., 3., 11.]),
        ];
        let a = find_risks(&boxes(&list));
        let reversed: Vec<_> = list.iter().rev().copied().collect();
        let b = find_risks(&boxes(&reversed));
        let pos = |r: &[Risk]| -> Vec<Vec3> {
            r.iter()
                .filter(|r| r.kind == RiskKind::Overhang)
                .map(|r| r.position)
                .collect()
        };
        assert_eq!(pos(&a).len(), 2);
        assert_eq!(pos(&a), pos(&b));
    }

    /// A downward-facing ramp: `run` along x, `len` along y, rising `rise`
    /// over the run from height `z0`. Its normal points down.
    fn ramp(w: &mut Welder, run: f32, len: f32, rise: f32, z0: f32) {
        let (a, b) = ([0., 0., z0], [run, 0., z0 + rise]);
        let (c, d) = ([run, len, z0 + rise], [0., len, z0]);
        w.push_triangle(a, c, b);
        w.push_triangle(a, d, c);
    }

    /// A big downward floor at z = 0, so the bed sits where the test expects.
    fn floor(w: &mut Welder) {
        let p = |x: f32, y: f32| [x, y, 0.];
        w.push_triangle(p(-50., -50.), p(50., 50.), p(50., -50.));
        w.push_triangle(p(-50., -50.), p(-50., 50.), p(50., 50.));
    }

    fn underside_at(degrees_from_horizontal: f32, z0: f32) -> Mesh {
        let mut w = Welder::default();
        floor(&mut w);
        let rise = 10.0 * degrees_from_horizontal.to_radians().tan();
        ramp(&mut w, 10.0, 10.0, rise, z0);
        w.finish()
    }

    #[test]
    fn only_undersides_within_45_degrees_of_flat_overhang() {
        // The normal's z is -cos(angle from horizontal): a ceiling-like 30 or
        // 40 degrees needs support; a wall-like 50 or 60 does not.
        for (degrees, overhangs) in [(0.0, true), (30.0, true), (40.0, true)]
            .into_iter()
            .chain([(50.0, false), (60.0, false), (80.0, false)])
        {
            let found = of_kind(find_risks(&underside_at(degrees, 5.0)), RiskKind::Overhang);
            assert_eq!(!found.is_empty(), overhangs, "{degrees} degrees: {found:?}");
        }
    }

    #[test]
    fn upward_faces_are_never_overhangs() {
        // The same 30 degree ramp, wound the other way up.
        let mut w = Welder::default();
        floor(&mut w);
        w.push_triangle([0., 0., 5.], [10., 0., 10.], [10., 10., 10.]);
        w.push_triangle([0., 0., 5.], [10., 10., 10.], [0., 10., 5.]);
        assert!(of_kind(find_risks(&w.finish()), RiskKind::Overhang).is_empty());
    }

    #[test]
    fn a_ceiling_inside_the_bed_band_rests_on_the_bed() {
        let ceiling = |z| {
            let mut w = Welder::default();
            floor(&mut w);
            ramp(&mut w, 10.0, 10.0, 0.0, z);
            of_kind(find_risks(&w.finish()), RiskKind::Overhang)
        };
        assert!(ceiling(0.2).is_empty());
        let high = ceiling(0.5);
        assert_eq!(high.len(), 1);
        assert!((high[0].value - 100.0).abs() < 0.01);
    }

    #[test]
    fn the_bed_is_the_models_lowest_point_not_zero() {
        // A block floating with its base at z = 5 still sits on its own bed.
        let risks = find_risks(&boxes(&[([0., 0., 5.], [20., 20., 15.])]));
        assert!(risks.is_empty(), "{risks:?}");
        // And a lifted T-shape has the same overhangs, at its own height.
        let t = boxes(&[
            ([8., 0., 100.], [12., 4., 112.]),
            ([0., 0., 110.], [8., 4., 112.]),
            ([12., 0., 110.], [20., 4., 112.]),
        ]);
        let over = of_kind(find_risks(&t), RiskKind::Overhang);
        assert_eq!(over.len(), 2, "{over:?}");
        assert!(over.iter().all(|r| (r.position[2] - 110.0).abs() < 0.01));
    }

    #[test]
    fn an_inverted_pyramid_is_an_overhang_not_bed_contact() {
        // Apex on the bed, a 80 mm square at z = 10: each side has one
        // vertex in the bed band and two far above it, and faces steeply down
        // (n.z about -0.97).
        let ring = [
            [40., 40., 10.],
            [-40., 40., 10.],
            [-40., -40., 10.],
            [40., -40., 10.],
        ];
        let mut w = Welder::default();
        for i in 0..4 {
            w.push_triangle([0., 0., 0.], ring[(i + 1) % 4], ring[i]);
        }
        w.push_triangle(ring[0], ring[1], ring[2]);
        w.push_triangle(ring[0], ring[2], ring[3]);
        let risks = find_risks(&w.finish());
        let over = of_kind(risks.clone(), RiskKind::Overhang);
        assert_eq!(over.len(), 1, "the four sides join into one: {over:?}");
        // Each side is 80 x sqrt(40^2 + 10^2) / 2 = 1649 mm2.
        assert!((over[0].value - 4.0 * 1649.24).abs() < 5.0, "{over:?}");
        // Nothing lies on the bed.
        let contact = of_kind(risks, RiskKind::SmallBedContact);
        assert_eq!(contact.len(), 1);
        assert_eq!(contact[0].value, 0.0);
    }

    #[test]
    fn only_nearly_flat_faces_on_the_bed_count_as_contact() {
        // Strips lying in the bed band. At n.z = -0.8 the face leans too far
        // to be contact; at -0.95 it counts, and 95 mm2 is plenty.
        let strip = |run: f32, rise: f32| {
            let mut w = Welder::default();
            ramp(&mut w, run, 100.0, rise, 0.0);
            of_kind(find_risks(&w.finish()), RiskKind::SmallBedContact)
        };
        let steep = strip(0.4, 0.3);
        assert_eq!(steep.len(), 1, "{steep:?}");
        assert_eq!(steep[0].value, 0.0);
        let gentle = strip(0.9, 0.296);
        assert!(gentle.is_empty(), "{gentle:?}");
    }

    #[test]
    fn a_big_plate_on_a_small_foot_may_lift() {
        let plate_on_foot = |foot: f32| {
            let (lo, hi) = (50. - foot / 2., 50. + foot / 2.);
            let m = boxes(&[
                ([lo, lo, 0.], [hi, hi, 5.]),
                ([0., 0., 5.], [100., 100., 6.]),
            ]);
            of_kind(find_risks(&m), RiskKind::SmallBedContact)
        };
        // 36 mm2 clears 20 but is under 5% of the 100 x 100 footprint.
        let small = plate_on_foot(6.0);
        assert_eq!(small.len(), 1, "{small:?}");
        assert!((small[0].value - 36.0).abs() < 0.01);
        // 900 mm2 is 9% of it, and is fine.
        assert!(plate_on_foot(30.0).is_empty());
    }

    #[test]
    fn modest_contact_over_twenty_mm2_is_fine() {
        let m = boxes(&[([0., 0., 0.], [10., 10., 2.])]);
        assert!(find_risks(&m).is_empty());
    }

    #[test]
    fn tall_and_thin_is_height_over_the_narrower_width() {
        let tower = |x: f32, y: f32, h: f32| {
            of_kind(
                find_risks(&boxes(&[([0., 0., 0.], [x, y, h])])),
                RiskKind::TallThin,
            )
        };
        // 70 over the narrower 10 is 7:1; over the wider 40 it would not be.
        assert_eq!(tower(10., 40., 70.).len(), 1);
        assert_eq!(tower(40., 10., 70.).len(), 1);
        // Either side of 5:1; exactly 5:1 is not tall and thin.
        assert!(tower(10., 10., 49.).is_empty());
        assert!(tower(10., 10., 50.).is_empty());
        let tall = tower(10., 10., 51.);
        assert_eq!(tall.len(), 1);
        assert!((tall[0].value - 5.1).abs() < 1e-4);
        assert_eq!(tall[0].position, [5., 5., 51.]);
        assert!(tall[0].message.starts_with("Tall and thin · 5:1"));
        // A 3:1 part is not.
        assert!(tower(10., 10., 30.).is_empty());
    }

    #[test]
    fn a_sliver_does_not_bridge_overhang_regions() {
        // Two ceilings at z = 5 meet a downward needle triangle along its two
        // short sides. The needle is so thin its normal is rounding noise
        // (-z here), so it must not link them into one region.
        let (p0, p1, p2) = ([0., 0., 5.], [100., 0., 5.], [50., 5e-5, 5.]);
        let mut w = Welder::default();
        floor(&mut w);
        w.push_triangle(p0, p2, p1);
        w.push_triangle(p0, p2, [25., -5., 5.]);
        w.push_triangle(p2, p1, [75., -5., 5.]);
        let over = of_kind(find_risks(&w.finish()), RiskKind::Overhang);
        assert_eq!(over.len(), 2, "{over:?}");
        assert!(over.iter().all(|r| (r.value - 125.0).abs() < 0.5));
    }

    #[test]
    fn a_sliver_on_the_bed_is_not_contact() {
        // A downward needle at z = 0, with a roof above it.
        let mut w = Welder::default();
        w.push_triangle([0., 0., 0.], [50., 5e-5, 0.], [100., 0., 0.]);
        w.push_triangle([0., 0., 3.], [10., 0., 3.], [0., 10., 3.]);
        let risks = find_risks(&w.finish());
        assert!(risks.iter().all(|r| r.kind != RiskKind::Overhang));
        let contact = of_kind(risks, RiskKind::SmallBedContact);
        assert_eq!(contact.len(), 1);
        assert_eq!(contact[0].value, 0.0);
    }

    #[test]
    fn risks_serialise_with_snake_case_kinds() {
        let r = Risk {
            kind: RiskKind::SmallBedContact,
            position: [0., 0., 0.],
            value: 1.0,
            message: String::new(),
        };
        assert!(serde_json::to_string(&r)
            .unwrap()
            .contains("\"kind\":\"small_bed_contact\""));
    }
}
