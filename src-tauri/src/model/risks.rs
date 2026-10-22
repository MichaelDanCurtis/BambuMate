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

/// Overhang regions (faces joined by shared edges) of at least
/// [`MIN_OVERHANG_AREA_MM2`], largest first, at most [`MAX_OVERHANG_REGIONS`].
/// Each is its area and area-weighted centroid.
fn overhang_regions(mesh: &Mesh, normals: &[Vec3], min_z: f32) -> Vec<(f64, Vec3)> {
    let lowest = |i: usize| {
        let [a, b, c] = mesh.triangles[i];
        [a, b, c]
            .into_iter()
            .map(|v| mesh.positions[v as usize][2])
            .fold(f32::INFINITY, f32::min)
    };
    let is_overhang = |i: usize| {
        has_normal(normals[i]) && normals[i][2] < OVERHANG_MAX_NZ && lowest(i) > min_z + BED_ZONE_MM
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

    // Faces lying on the bed: facing down, every corner within the bed zone.
    let mut contact = 0f64;
    let mut weighted = [0f64; 3];
    for (i, &n) in normals.iter().enumerate() {
        if !has_normal(n) || n[2] >= BED_CONTACT_MAX_NZ {
            continue;
        }
        let [a, b, c] = mesh.tri(i);
        if [a, b, c].iter().any(|p| p[2] > lo[2] + BED_ZONE_MM) {
            continue;
        }
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

    #[test]
    fn sliver_faces_are_never_overhangs_or_bed_contact() {
        // A long needle triangle pointing straight down at the bed, plus a
        // floor and a roof so the bounds are sensible. The needle's area is
        // tiny and its normal is noise.
        let mut w = Welder::default();
        w.push_triangle([0., 0., 0.], [100., 0., 0.], [50., 1e-5, 0.]);
        w.push_triangle([0., 0., 5.], [100., 0., 5.], [50., 0., 6.]);
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
