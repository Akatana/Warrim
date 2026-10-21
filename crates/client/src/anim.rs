//! M2 skeletal animation: evaluate bone tracks for a sequence at a time, and skin vertices.

use crate::models::ModelVertex;
use glam::{Mat4, Quat, Vec3};
use wowfmt::m2::{M2Track, M2};

/// Value of `track` for sequence `seq` at `time` ms on the model's timeline (`global_ms` drives
/// tracks tied to a global sequence). `None` if the track has no key for the sequence.
pub(crate) fn sample<T: Copy>(track: &M2Track<T>, seq: usize, time: f32, global_ms: &[u32], global_time: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
    if track.keys.is_empty() || track.times.is_empty() {
        return None;
    }
    let (mut first, mut last, mut t) = (0, track.times.len() - 1, time);
    if track.global_sequence >= 0 {
        let len = *global_ms.get(track.global_sequence as usize)? as f32;
        t = if len > 0.0 { global_time.rem_euclid(len) } else { 0.0 };
        if let Some(&(f, l)) = track.ranges.first() {
            (first, last) = (f as usize, l as usize);
        }
    } else if track.ranges.is_empty() {
        // A value that does not change with the animation (a particle emitter's speed): its keys are all there is.
    } else {
        let &(f, l) = track.ranges.get(seq)?;
        (first, last) = (f as usize, l as usize);
    }
    last = last.min(track.times.len() - 1);
    if first > last {
        return None;
    }
    // Hermite and Bezier keys carry two tangents after each value: use the values only.
    let stride = (track.keys.len() / track.times.len()).max(1);
    let key = |i: usize| track.keys.get(i * stride).copied();
    if t <= track.times[first] as f32 {
        return key(first);
    }
    if t >= track.times[last] as f32 {
        return key(last);
    }
    let i = (first..last).rev().find(|&i| track.times[i] as f32 <= t)?;
    let (t0, t1) = (track.times[i] as f32, track.times[i + 1] as f32);
    let (a, b) = (key(i)?, key(i + 1)?);
    Some(if track.interpolation == 0 { a } else { lerp(a, b, ((t - t0) / (t1 - t0).max(1.0)).clamp(0.0, 1.0)) })
}

/// A sequence at a time (ms on the model's timeline).
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub seq: usize,
    pub time: f32,
}

/// What a bone plays: the pose `to`, faded in from `from` (`weight` is how much of `to` shows).
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    pub from: Option<Pose>,
    pub to: Pose,
    pub weight: f32,
}

impl Mix {
    pub fn one(seq: usize, time: f32) -> Mix {
        Mix { from: None, to: Pose { seq, time }, weight: 1.0 }
    }
}

/// Model-space matrix of every bone for sequence `seq` at `time` ms.
pub fn bone_matrices(m2: &M2, seq: usize, time: f32, global_time: f32) -> Vec<Mat4> {
    bone_matrices_with(m2, Mix::one(seq, time), global_time, &[], &[])
}

/// Like `bone_matrices`, but every bone plays `mix` (a cross-fade between two poses) unless it has a `Some`
/// entry in `overrides`, which is used as its matrix instead of its animation (its children follow it, e.g.
/// arms posed by inverse kinematics), or in `poses`, which makes it play that mix instead (closed hands,
/// the upper body's own animation).
pub fn bone_matrices_with(m2: &M2, mix: Mix, global_time: f32, overrides: &[Option<Mat4>], poses: &[Option<Mix>]) -> Vec<Mat4> {
    bone_matrices_seen_from(m2, mix, global_time, overrides, poses, None)
}

/// Like `bone_matrices`, for a model seen from `eye` (model space): its billboards turn to face it.
pub fn bone_matrices_facing(m2: &M2, seq: usize, time: f32, global_time: f32, eye: Vec3) -> Vec<Mat4> {
    bone_matrices_seen_from(m2, Mix::one(seq, time), global_time, &[], &[], Some(eye))
}

/// Whether some bones of the model are billboards (flames, glows), which must be turned to face the viewer.
pub fn has_billboards(m2: &M2) -> bool {
    m2.bones.iter().any(|b| b.flags & BILLBOARD != 0)
}

/// The bone flags that make a bone a billboard: it faces the viewer (0x8 all round, 0x10, 0x20 and 0x40 turning about one axis).
const BILLBOARD: u32 = 0x78;

/// The turn that makes a billboard at `pivot` face `eye` (both in model space): its x axis points at the viewer, and its z axis stays up; a
/// billboard that may turn about the vertical only (0x40) looks level.
fn facing(flags: u32, pivot: Vec3, eye: Vec3) -> Mat4 {
    let mut to = eye - pivot;
    if flags & 0x40 != 0 {
        to.z = 0.0;
    }
    let x = to.normalize_or_zero();
    if x == Vec3::ZERO {
        return Mat4::IDENTITY;
    }
    let up = if x.z.abs() > 0.999 { Vec3::Y } else { Vec3::Z };
    let z = (up - x * up.dot(x)).normalize();
    Mat4::from_cols(x.extend(0.0), z.cross(x).extend(0.0), z.extend(0.0), glam::Vec4::W)
}

fn bone_matrices_seen_from(m2: &M2, mix: Mix, global_time: f32, overrides: &[Option<Mat4>], poses: &[Option<Mix>], eye: Option<Vec3>) -> Vec<Mat4> {
    let n = m2.bones.len();
    let mut out: Vec<Option<Mat4>> = overrides.to_vec();
    out.resize(n, None);
    #[allow(clippy::too_many_arguments)]
    fn eval(m2: &M2, b: usize, mix0: Mix, global_time: f32, poses: &[Option<Mix>], out: &mut Vec<Option<Mat4>>, depth: usize, eye: Option<Vec3>) -> Mat4 {
        if let Some(m) = out[b] {
            return m;
        }
        let mix = poses.get(b).copied().flatten().unwrap_or(mix0);
        let bone = &m2.bones[b];
        let g = &m2.global_sequences;
        let at = |p: Pose| {
            let t = sample(&bone.translation, p.seq, p.time, g, global_time, |a, b, f| Vec3::from(a).lerp(Vec3::from(b), f).to_array()).map_or(Vec3::ZERO, Vec3::from);
            let r = sample(&bone.rotation, p.seq, p.time, g, global_time, |a, b, f| {
                let (qa, qb) = (Quat::from_array(a), Quat::from_array(b));
                // Shortest path.
                qa.slerp(if qa.dot(qb) < 0.0 { -qb } else { qb }, f).to_array()
            })
            .map_or(Quat::IDENTITY, |q| Quat::from_array(q).normalize());
            let s = sample(&bone.scale, p.seq, p.time, g, global_time, |a, b, f| Vec3::from(a).lerp(Vec3::from(b), f).to_array()).map_or(Vec3::ONE, Vec3::from);
            (t, r, s)
        };
        let (mut t, mut r, mut s) = at(mix.to);
        if let Some(from) = mix.from.filter(|_| mix.weight < 1.0) {
            let (t0, r0, s0) = at(from);
            let w = mix.weight.max(0.0);
            (t, r, s) = (t0.lerp(t, w), r0.slerp(if r0.dot(r) < 0.0 { -r } else { r }, w), s0.lerp(s, w));
        }
        let pivot = Vec3::from(bone.pivot);
        let turn = match eye {
            Some(eye) if bone.flags & BILLBOARD != 0 => facing(bone.flags, pivot + t, eye),
            _ => Mat4::from_quat(r),
        };
        let local = Mat4::from_translation(pivot) * Mat4::from_translation(t) * turn * Mat4::from_scale(s) * Mat4::from_translation(-pivot);
        let parent = bone.parent;
        let m = if parent >= 0 && (parent as usize) < m2.bones.len() && depth < 64 {
            eval(m2, parent as usize, mix0, global_time, poses, out, depth + 1, eye) * local
        } else {
            local
        };
        out[b] = Some(m);
        m
    }
    (0..n).map(|b| eval(m2, b, mix, global_time, poses, &mut out, 0, eye)).collect()
}

/// Skin the model's vertices with `bones`, into render space (x east, y up, z south).
pub fn skin(m2: &M2, bones: &[Mat4], template: &[ModelVertex], out: &mut Vec<ModelVertex>) {
    out.clear();
    out.extend(m2.vertices.iter().zip(template).map(|(v, base)| {
        let (p, n) = (Vec3::from(v.pos), Vec3::from(v.normal));
        let (mut pos, mut normal, mut total) = (Vec3::ZERO, Vec3::ZERO, 0.0);
        for k in 0..4 {
            let w = v.weights[k] as f32 / 255.0;
            if w == 0.0 {
                continue;
            }
            let m = bones.get(v.bones[k] as usize).copied().unwrap_or(Mat4::IDENTITY);
            pos += m.transform_point3(p) * w;
            normal += m.transform_vector3(n) * w;
            total += w;
        }
        if total == 0.0 {
            (pos, normal) = (p, n);
        }
        let n = normal.normalize_or_zero();
        ModelVertex { pos: [pos.x, pos.z, -pos.y], normal: [n.x, n.z, -n.y], uv: base.uv, color: base.color }
    }));
}

/// Which of the animated looks of a model a batch has.
#[derive(Clone, Copy, Debug, Default)]
pub struct BatchRefs {
    pub color: Option<usize>,
    pub transparency: Option<usize>,
    pub uv: Option<usize>,
}

/// What a batch looks like at a moment: its colour, how opaque it is, and where its texture has moved (two rows of a matrix on the texture's
/// coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatchLook {
    pub tint: [f32; 3],
    pub opacity: f32,
    pub uv: [[f32; 3]; 2],
}

impl BatchLook {
    pub const STILL: BatchLook = BatchLook { tint: [1.0; 3], opacity: 1.0, uv: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] };
}

/// The tracks that animate how the batches of a model look (colours, transparency, the movement of textures), and the animation they follow: the
/// stand sequence, over and over.
#[derive(Debug)]
pub struct LookTracks {
    colors: Vec<wowfmt::m2::M2Color>,
    transparency: Vec<M2Track<f32>>,
    uv: Vec<wowfmt::m2::M2UvAnim>,
    global_sequences: Vec<u32>,
    seq: usize,
    start: f32,
    length: f32,
    /// Some track of a batch changes over time: only then does the look need writing again each frame (the rest is as it is at the start).
    pub animated: bool,
}

impl LookTracks {
    /// The tracks of the model if any batch uses one.
    pub fn of(m2: &M2) -> Option<std::sync::Arc<LookTracks>> {
        if !m2.batches.iter().any(|b| b.color.is_some() || b.transparency.is_some() || b.uv_anim.is_some()) {
            return None;
        }
        let seq = m2.sequences.iter().position(|s| s.id == 0).unwrap_or(0);
        let (start, length) = m2.sequences.get(seq).map_or((0.0, 1000.0), |s| (s.start as f32, ((s.end - s.start) as f32).max(1.0)));
        // A track with one key (or none) says the same all through.
        let moves = |keys: usize| keys > 1;
        let used = |b: &wowfmt::m2::M2Batch| {
            b.color.is_some_and(|i| moves(m2.colors[i].color.keys.len()) || moves(m2.colors[i].alpha.keys.len()))
                || b.transparency.is_some_and(|i| moves(m2.transparency[i].keys.len()))
                || b.uv_anim.is_some_and(|i| moves(m2.uv_anims[i].translation.keys.len()) || moves(m2.uv_anims[i].rotation.keys.len()) || moves(m2.uv_anims[i].scale.keys.len()))
        };
        let animated = m2.batches.iter().any(used);
        Some(std::sync::Arc::new(LookTracks { colors: m2.colors.clone(), transparency: m2.transparency.clone(), uv: m2.uv_anims.clone(), global_sequences: m2.global_sequences.clone(), seq, start, length, animated }))
    }

    /// The look of a batch `age_ms` after its model began to play.
    pub fn look(&self, refs: BatchRefs, age_ms: f32) -> BatchLook {
        self.look_in(refs, self.seq, self.start + age_ms.rem_euclid(self.length), age_ms)
    }

    /// The look of a batch while sequence `seq` of the model is at `time` (milliseconds on the model's own clock: its start plus the time into it), `age_ms` after
    /// the model began (for the tracks that run on the global clock): what a creature that plays its own animations shows (a log it carries only in some of them).
    pub fn look_in(&self, refs: BatchRefs, seq: usize, time: f32, age_ms: f32) -> BatchLook {
        let g = &self.global_sequences[..];
        let lerp3 = |a: [f32; 3], b: [f32; 3], f: f32| Vec3::from(a).lerp(Vec3::from(b), f).to_array();
        let lerp1 = |a: f32, b: f32, f: f32| a + (b - a) * f;
        let mut look = BatchLook::STILL;
        if let Some(c) = refs.color.and_then(|i| self.colors.get(i)) {
            if let Some(rgb) = sample(&c.color, seq, time, g, age_ms, lerp3) {
                look.tint = rgb;
            }
            if let Some(a) = sample(&c.alpha, seq, time, g, age_ms, lerp1) {
                look.opacity *= a;
            }
        }
        if let Some(t) = refs.transparency.and_then(|i| self.transparency.get(i)) {
            if let Some(a) = sample(t, seq, time, g, age_ms, lerp1) {
                look.opacity *= a;
            }
        }
        if let Some(u) = refs.uv.and_then(|i| self.uv.get(i)) {
            let shift = sample(&u.translation, seq, time, g, age_ms, lerp3).unwrap_or([0.0; 3]);
            let scale = sample(&u.scale, seq, time, g, age_ms, lerp3).unwrap_or([1.0; 3]);
            let turn = sample(&u.rotation, seq, time, g, age_ms, |a, b, f| {
                let (qa, qb) = (Quat::from_array(a), Quat::from_array(b));
                qa.slerp(if qa.dot(qb) < 0.0 { -qb } else { qb }, f).to_array()
            })
            .map_or(0.0, |q| 2.0 * q[2].atan2(q[3]));
            // The texture turns and grows about its middle, then moves.
            let (sin, cos) = turn.sin_cos();
            let (a, b, c, d) = (cos * scale[0], -sin * scale[1], sin * scale[0], cos * scale[1]);
            look.uv = [[a, b, 0.5 - (a + b) * 0.5 + shift[0]], [c, d, 0.5 - (c + d) * 0.5 + shift[1]]];
        }
        look
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(times: &[u32], keys: &[[f32; 3]]) -> M2Track<[f32; 3]> {
        M2Track { interpolation: 1, global_sequence: -1, ranges: vec![(0, times.len() as u32 - 1)], times: times.into(), keys: keys.into() }
    }

    #[test]
    fn sampling_interpolates_clamps_and_respects_the_sequence_range() {
        let lerp = |a: [f32; 3], b: [f32; 3], f: f32| Vec3::from(a).lerp(Vec3::from(b), f).to_array();
        let t = track(&[100, 200, 300], &[[0.0; 3], [10.0, 0.0, 0.0], [10.0, 20.0, 0.0]]);
        let at = |time| sample(&t, 0, time, &[], 0.0, lerp).unwrap();
        assert_eq!(at(150.0), [5.0, 0.0, 0.0]);
        assert_eq!(at(0.0), [0.0; 3]);
        assert_eq!(at(999.0), [10.0, 20.0, 0.0]);
        // A sequence without a range has no key.
        assert!(sample(&t, 5, 150.0, &[], 0.0, lerp).is_none());
    }

    #[test]
    fn the_two_layers_of_a_waterfall_scroll_at_their_own_speeds() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let m2 = M2::parse(&mount.read(r"World\Azeroth\Elwynn\PassiveDoodads\Waterfall\ElwynnTallWaterfall01.m2").unwrap()).unwrap();
        assert_eq!(m2.uv_anims.len(), 2, "its texture animations are read from the right array of the header");
        let tracks = LookTracks::of(&m2).expect("the model animates its pictures");
        let shift = |batch: usize, ms: f32| tracks.look(BatchRefs { color: None, transparency: None, uv: m2.batches[batch].uv_anim }, ms).uv[1][2];
        // One unit down in ten seconds for the first layer, two for the second.
        let (a, b) = (shift(0, 5000.0) - shift(0, 1000.0), shift(1, 5000.0) - shift(1, 1000.0));
        assert!(a < -0.3 && (b / a - 2.0).abs() < 0.05, "{a} {b}");
    }

    #[test]
    fn a_billboard_turns_its_front_to_the_viewer_and_keeps_its_top_up() {
        let eye = Vec3::new(3.0, 4.0, 5.0);
        let pivot = Vec3::new(1.0, 1.0, 1.0);
        let m = facing(0x8, pivot, eye);
        let to = (eye - pivot).normalize();
        assert!((m.transform_vector3(Vec3::X) - to).length() < 1e-5, "its front looks at the viewer");
        assert!(m.transform_vector3(Vec3::Z).z > 0.5, "its top is up");
        // One that turns about the vertical only looks level.
        assert!(facing(0x40, pivot, eye).transform_vector3(Vec3::X).z.abs() < 1e-5);
    }

    #[test]
    fn the_rings_of_a_level_up_fade_in_and_out_by_their_own_tracks_and_are_tinted_gold() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let m2 = M2::parse(&mount.read(r"Spells\LevelUp\LevelUp.m2").unwrap()).unwrap();
        let tracks = LookTracks::of(&m2).expect("the model has tracks");
        let ring = BatchRefs { transparency: m2.batches[0].transparency, ..Default::default() };
        let (start, middle, end) = (tracks.look(ring, 0.0).opacity, tracks.look(ring, 700.0).opacity, tracks.look(ring, 1500.0).opacity);
        assert!(start < 0.05 && middle > 0.95 && end < 0.05, "{start} {middle} {end}");
        // The last batch is gold: blue nothing, red and green most.
        let last = m2.batches.last().unwrap();
        let gold = tracks.look(BatchRefs { color: last.color, ..Default::default() }, 0.0).tint;
        assert!(gold[0] > 0.8 && gold[1] > 0.7 && gold[2] < 0.05, "{gold:?}");
        // A model's own still values (a fireball's) are just as good.
        assert_eq!(BatchLook::STILL.opacity, 1.0);
    }
}
