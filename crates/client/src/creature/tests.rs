use super::*;

fn kind() -> usize {
    content::creature_index("wolf")
}

fn def() -> &'static CreatureDef {
    content::creature("wolf")
}

fn flat_ground(_: f32, _: f32, _: f32) -> Option<f32> {
    Some(0.0)
}

fn open(_: Vec3, _: Vec3) -> bool {
    false
}

/// Run `seconds` at 60 fps against a player standing at `target`; collects the damage dealt.
fn run(mob: &mut Mob, seconds: f32, target: Vec3) -> Vec<f32> {
    (0..(seconds * 60.0) as usize).filter_map(|_| mob.update(1.0 / 60.0, target, flat_ground, open)).collect()
}

#[test]
fn a_wolfs_name_floats_at_its_attachment_and_a_model_without_one_at_a_fixed_height() {
    let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
    let Body::Beast { model, .. } = &def().body else { panic!("a wolf is a beast") };
    let mut m2 = M2::parse(&mount.read(model).unwrap()).unwrap();
    let bones = vec![Mat4::IDENTITY; m2.bones.len()];
    let up = name_offset(&m2, &bones, Mat4::IDENTITY, 1.0);
    assert!(up.y > 0.5 && up.y < 3.0, "{up:?}");
    m2.attachment_lookup[NAME_ATTACHMENT] = 0xFFFF;
    assert_eq!(name_offset(&m2, &bones, Mat4::IDENTITY, 2.0), Vec3::Y * 2.5);
}

#[test]
fn a_wolf_ignores_a_distant_player_then_chases_strikes_and_keeps_striking() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    assert!(run(&mut wolf, 3.0, Vec3::new(30.0, 0.0, 0.0)).is_empty() && wolf.state == State::Idle);
    // A player 10 yards east: it turns to face east and runs to him (6.94 yd/s: about 1.2 s to get in range).
    let player = Vec3::new(10.0, 0.0, 0.0);
    run(&mut wolf, 0.2, player);
    assert_eq!(wolf.state, State::Chase);
    assert!((wolf.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "yaw {}", wolf.yaw);
    let strikes = run(&mut wolf, 6.0, player);
    assert!((wolf.pos - player).length() < MELEE_RANGE, "stopped at {:?}", wolf.pos);
    // Over 6 seconds: a melee attack every 2.1 s (1.5 s animation and a pause).
    assert!(strikes.len() >= 2 && strikes.iter().all(|&d| d == def().melee_damage), "{strikes:?}");
}

#[test]
fn a_wolf_does_not_walk_through_a_wall_where_there_is_no_mesh_to_lead_it() {
    // A wall across x = 4, and a player behind it, 10 yards east and 3 north.
    let wall = |_: Vec3, to: Vec3| to.x > 4.0;
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let player = Vec3::new(10.0, 0.0, -3.0);
    for _ in 0..600 {
        wolf.update(1.0 / 60.0, player, flat_ground, wall);
    }
    assert!(wolf.pos.x <= 4.0, "went through: {:?}", wolf.pos);
}

#[test]
fn a_blow_misses_when_the_player_has_stepped_away_and_a_wolf_gives_up_far_away() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.state = State::Melee(0.0, false);
    // Moves 5 yards off while the teeth are coming: no damage.
    assert!(run(&mut wolf, 1.6, Vec3::new(5.0, 0.0, 0.0)).is_empty());
    wolf.state = State::Chase;
    run(&mut wolf, 0.1, Vec3::new(50.0, 0.0, 0.0));
    assert!(matches!(wolf.state, State::Return(_)));
}

/// A wound that bleeds, a stun that holds it, a slow that makes its time run slower.
#[test]
fn what_is_put_on_a_creature_hurts_holds_or_slows_it() {
    let far = Vec3::new(100.0, 0.0, 0.0);
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let full = wolf.health;
    wolf.afflict(Status::Over { amount: 3.0, period: 1.0, seconds: 5.0 });
    run(&mut wolf, 5.5, far);
    assert!((full - wolf.health - 15.0).abs() < 1e-3, "five ticks of three: {}", full - wolf.health);
    // A stun: it stands where it is for the seconds, then chases.
    wolf.state = State::Chase;
    wolf.pos = Vec3::new(10.0, 0.0, 0.0);
    wolf.afflict(Status::Stun(1.0));
    run(&mut wolf, 0.9, Vec3::ZERO);
    assert_eq!(wolf.pos.x, 10.0);
    run(&mut wolf, 0.6, Vec3::ZERO);
    assert!(wolf.pos.x < 10.0);
    // A slow to a share of the pace: it covers that share of the ground.
    let (mut quick, mut slow) = (Mob::new(kind(), Vec3::new(30.0, 0.0, 0.0), 0.0), Mob::new(kind(), Vec3::new(30.0, 0.0, 0.0), 0.0));
    (quick.state, slow.state) = (State::Chase, State::Chase);
    slow.afflict(Status::Slow(0.5, 10.0));
    run(&mut quick, 1.0, Vec3::ZERO);
    run(&mut slow, 1.0, Vec3::ZERO);
    assert!(((30.0 - slow.pos.x) / (30.0 - quick.pos.x) - 0.5).abs() < 0.1, "{} {}", slow.pos.x, quick.pos.x);
    // Damage over time can kill, and says so once.
    let mut weak = Mob::new(kind(), Vec3::ZERO, 0.0);
    weak.health = 2.0;
    weak.afflict(Status::Over { amount: 5.0, period: 1.0, seconds: 5.0 });
    run(&mut weak, 1.5, far);
    assert!(weak.is_dead() && std::mem::take(&mut weak.slain_over) && !weak.slain_over);
}

#[test]
fn a_wolf_that_gave_up_waits_then_runs_home_and_heals_unless_hit_again() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.pos = Vec3::new(30.0, 0.0, 0.0);
    wolf.health = 10.0;
    wolf.state = State::Chase;
    let far = Vec3::new(100.0, 0.0, 0.0);
    run(&mut wolf, 0.1, far);
    assert!(matches!(wolf.state, State::Return(_)));
    // It stands still for a while, hurt...
    run(&mut wolf, RETURN_WAIT - 0.5, far);
    assert_eq!((wolf.pos.x, wolf.health), (30.0, 10.0));
    // ...a hit in that time turns it back on the attacker...
    let mut hit = Mob::new(kind(), Vec3::ZERO, 0.0);
    hit.state = State::Return(1.0);
    hit.hurt(1.0);
    assert!(matches!(hit.state, State::Flinch(_)));
    // ...otherwise it runs home and is whole again.
    run(&mut wolf, 8.0, far);
    assert_eq!((wolf.state, wolf.health), (State::Idle, def().health));
    assert!(wolf.pos.x < 0.3, "{:?}", wolf.pos);
}

#[test]
fn hits_flinch_then_kill_and_the_wolf_returns_after_a_while() {
    let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
    wolf.hurt(12.0);
    assert_eq!((wolf.state, wolf.health), (State::Flinch(0.0), 38.0));
    // It flinches (no movement), then chases the one who hit it from far away... within the leash.
    run(&mut wolf, 0.3, Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(wolf.pos.x, 3.0);
    run(&mut wolf, 0.3, Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(wolf.state, State::Chase);
    (0..4).for_each(|_| wolf.hurt(12.0));
    assert!(wolf.is_dead() && wolf.health < 0.0);
    // A corpse neither moves nor strikes, and hits on it do nothing.
    assert!(run(&mut wolf, 10.0, Vec3::new(3.5, 0.0, 0.0)).is_empty() && wolf.is_dead());
    wolf.hurt(5.0);
    // After the respawn time it stands at home again with full health.
    run(&mut wolf, def().respawn - 9.0, Vec3::new(500.0, 0.0, 0.0));
    assert_eq!((wolf.state, wolf.health, wolf.pos), (State::Idle, def().health, Vec3::new(3.0, 0.0, 0.0)));
}

#[test]
fn a_saved_creature_comes_back_as_it_was_and_only_to_its_own_spawn() {
    let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.5);
    wolf.hurt(def().health + 1.0);
    wolf.pos = Vec3::new(4.0, 0.0, 1.0);
    wolf.state = State::Dead(12.5);
    let saved: MobSave = ron::from_str(&ron::to_string(&wolf.snapshot()).unwrap()).unwrap();
    let mut fresh = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
    assert!(fresh.restore(&saved));
    assert_eq!(fresh.snapshot(), wolf.snapshot());
    assert!(fresh.is_dead() && fresh.pos == Vec3::new(4.0, 0.0, 1.0));
    // Another spawn (a different home) is not the same creature.
    assert!(!Mob::new(kind(), Vec3::new(50.0, 0.0, 0.0), 0.0).restore(&saved));
}

#[test]
fn a_swing_hits_what_is_in_front_and_in_reach_only() {
    // The player at the origin faces north (-Z).
    let hit = |at: Vec3| Mob::new(kind(), at, 0.0).struck_by(Vec3::ZERO, 0.0, 3.5, 12.0);
    assert!(hit(Vec3::new(0.0, 0.0, -3.0)));
    assert!(!hit(Vec3::new(0.0, 0.0, -4.0)), "out of reach");
    assert!(!hit(Vec3::new(0.0, 0.0, 3.0)), "behind");
    assert!(!hit(Vec3::new(3.0, 0.0, -0.5)), "to the side");
    assert!(hit(Vec3::new(1.5, 0.0, -2.5)), "within the cone");
    let mut dead = Mob::new(kind(), Vec3::new(0.0, 0.0, -2.0), 0.0);
    dead.state = State::Dead(0.0);
    assert!(!dead.struck_by(Vec3::ZERO, 0.0, 3.5, 12.0));
}

#[test]
fn a_wolf_cries_when_it_notices_snarls_and_strikes_is_wounded_and_dies() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let player = Vec3::new(10.0, 0.0, 0.0);
    let mut heard = Vec::new();
    for _ in 0..(5.0 * 60.0) as usize {
        wolf.update(1.0 / 60.0, player, flat_ground, open);
        heard.append(&mut wolf.sounds);
    }
    // Paws hit the ground while it runs (8 yards: three or so steps).
    let steps = heard.iter().filter(|&&s| s == Cry::Footstep).count();
    assert!((3..=4).contains(&steps), "{steps} footsteps");
    heard.retain(|&s| s != Cry::Footstep);
    // Aggro once, then a snarl and the melee attack's impact for each melee attack.
    assert_eq!(heard[..3], [Cry::Aggro, Cry::Attack, Cry::CustomAttack]);
    assert_eq!(heard.iter().filter(|&&s| s == Cry::Aggro).count(), 1);
    wolf.hurt(12.0);
    assert_eq!(wolf.sounds.pop(), Some(Cry::Wound));
    (0..4).for_each(|_| wolf.hurt(12.0));
    assert_eq!(wolf.sounds.last(), Some(&Cry::Death));
    wolf.sounds.clear();
    wolf.hurt(12.0);
    assert!(wolf.sounds.is_empty(), "a corpse makes no sound");
}

#[test]
fn a_killed_wolf_holds_loot_until_it_comes_back() {
    let mut wolf = Mob::new(kind(), Vec3::new(3.0, 0.0, 0.0), 0.0);
    assert!(wolf.loot.is_none());
    wolf.hurt(12.0);
    assert!(wolf.loot.is_none(), "a living wolf has nothing to loot");
    (0..4).for_each(|_| wolf.hurt(12.0));
    assert!(wolf.is_dead() && wolf.loot.is_some());
    // Hits on a corpse do not roll again.
    let rolled = wolf.loot.clone();
    wolf.hurt(5.0);
    assert_eq!(wolf.loot, rolled);
    run(&mut wolf, def().respawn + 1.0, Vec3::new(500.0, 0.0, 0.0));
    assert!(wolf.loot.is_none() && !wolf.is_dead());
}

#[test]
fn a_corpse_out_of_reach_still_counts_down_to_the_day_it_is_back_and_only_what_changed_is_saved() {
    let mut mob = Mob::new(content::creature_index("c299"), Vec3::new(5.0, 0.0, 5.0), 0.0);
    assert!(!mob.worth_saving(), "one at rest starts afresh");
    mob.health -= 1.0;
    assert!(mob.worth_saving());
    (mob.state, mob.pos) = (State::Dead(0.0), Vec3::new(9.0, 0.0, 9.0));
    mob.tick_dead(mob.respawn - 1.0);
    assert!(matches!(mob.state, State::Dead(_)) && mob.worth_saving());
    mob.tick_dead(2.0);
    assert_eq!((mob.state, mob.pos, mob.health), (State::Idle, Vec3::new(5.0, 0.0, 5.0), mob.def.health));
    assert!(!mob.worth_saving());
}

#[test]
fn an_unbothered_hostile_creature_strolls_near_home_and_rests_between_walks() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let far = Vec3::new(500.0, 0.0, 0.0);
    let (mut walked, mut rested, mut farthest) = (false, false, 0.0f32);
    for _ in 0..(120.0 * 60.0) as usize {
        wolf.update(1.0 / 60.0, far, flat_ground, open);
        walked |= matches!(wolf.state, State::Wander(_));
        rested |= walked && wolf.state == State::Idle;
        farthest = farthest.max(flat(wolf.pos).length());
    }
    assert!(walked && rested, "it walks, then stands again");
    assert!(farthest > 1.0 && farthest <= WANDER_DEFAULT + 0.5, "strolled {farthest} yards from home");
}

fn stop(x: f32, z: f32, wait: f32, steps: Vec<(f32, content::Step)>) -> Stop {
    Stop { at: Vec3::new(x, 0.0, z), wait, facing: None, steps }
}

/// A guard (a friendly creature) with a patrol, run for `seconds` far from any player; calls `watch` after each step.
fn patrol(route: Vec<Stop>, seconds: f32, mut watch: impl FnMut(&Mob)) -> Mob {
    let mut guard = Mob::new(content::creature_index("c1642"), Vec3::ZERO, 0.0);
    guard.route = route;
    let far = Vec3::new(500.0, 0.0, 500.0);
    for _ in 0..(seconds * 60.0) as usize {
        guard.update(1.0 / 60.0, far, flat_ground, open);
        watch(&guard);
    }
    guard
}

#[test]
fn a_patrol_walks_its_places_in_turn_standing_and_facing_where_they_ask_and_comes_back_in_its_own_time() {
    // Three places, the second asking for ten seconds standing with the face turned.
    let mut route = vec![stop(10.0, 0.0, 0.0, vec![]), stop(10.0, 10.0, 10.0, vec![]), stop(0.0, 10.0, 0.0, vec![])];
    route[1].facing = Some(1.0);
    let (mut reached, mut stood) = ([false; 3], 0.0f32);
    let places: Vec<Vec3> = route.iter().map(|r| r.at).collect();
    patrol(route, 120.0, |g| {
        for (r, place) in reached.iter_mut().zip(&places) {
            *r |= flat(g.pos - *place).length() < 0.5;
        }
        if flat(g.pos - places[1]).length() < 0.5 && g.state == State::Idle {
            stood += 1.0 / 60.0;
            assert_eq!(g.yaw, 1.0, "it turns to the heading of the place while it stands");
        }
    });
    assert_eq!(reached, [true; 3], "every place of the patrol is reached");
    assert!(stood > 9.0, "it stands at the second place: {stood} s");
    // Its own respawn time is the one it waits.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.respawn = 20.0;
    wolf.state = State::Dead(0.0);
    run(&mut wolf, 19.0, Vec3::new(500.0, 0.0, 500.0));
    assert!(wolf.is_dead());
    run(&mut wolf, 2.0, Vec3::new(500.0, 0.0, 500.0));
    assert!(!wolf.is_dead());
}

#[test]
fn the_scripts_of_the_places_make_it_run_hold_an_animation_change_its_look_and_stay_where_it_is() {
    use content::Step;
    let wolf = content::creature_index("wolf");
    // The first place: run from here on at once, hold animation 136 after half a second, and look like a wolf after a second. The second: stay.
    let route = vec![stop(10.0, 0.0, 2.0, vec![(0.0, Step::Run(true)), (0.5, Step::Emote(136)), (1.0, Step::Morph("wolf".into()))]), stop(30.0, 0.0, 0.0, vec![(0.0, Step::Idle)])];
    let (mut emote_seen, mut look_seen, mut fastest) = (None, None, 0.0f32);
    let mut last = Vec3::ZERO;
    let guard = patrol(route, 40.0, |g| {
        if g.state == State::Idle && flat(g.pos - Vec3::new(10.0, 0.0, 0.0)).length() < 0.5 {
            emote_seen = emote_seen.or(g.emote);
            look_seen = look_seen.or((g.look == wolf).then_some(true));
        }
        fastest = fastest.max(flat(g.pos - last).length() * 60.0);
        last = g.pos;
    });
    assert_eq!((emote_seen, look_seen), (Some(136), Some(true)));
    assert!(fastest > WALK_SPEED * 1.5, "it ran between the places: {fastest}");
    assert!(guard.stopped && flat(guard.pos - Vec3::new(30.0, 0.0, 0.0)).length() < 0.5 && guard.state == State::Idle, "it stays at the last place");
    // Back from the dead it starts again, walking, as it looked.
    let mut guard = guard;
    guard.state = State::Dead(guard.respawn);
    run(&mut guard, 1.0, Vec3::new(500.0, 0.0, 500.0));
    assert_eq!((guard.next, guard.run, guard.stopped, guard.emote, guard.look), (0, false, false, None, guard.kind));
}

#[test]
fn a_script_can_have_it_stroll_round_a_place_for_a_while_before_the_patrol_goes_on() {
    let route = vec![stop(10.0, 0.0, 0.0, vec![(0.0, content::Step::Roam(6.0, 20.0))]), stop(40.0, 0.0, 0.0, vec![])];
    let (mut strayed, mut left_at) = (0.0f32, None);
    patrol(route, 60.0, |g| {
        if g.roam.is_some() {
            strayed = strayed.max(flat(g.pos - Vec3::new(10.0, 0.0, 0.0)).length());
        } else if g.next == 0 || left_at.is_some() {
        } else {
            left_at = Some(());
        }
    });
    assert!(strayed > 1.0 && strayed <= 6.5, "it strolled within six yards of the place: {strayed}");
    assert!(left_at.is_some(), "and then went on");
}

#[test]
fn how_far_along_its_patrol_a_creature_is_comes_back_with_the_save() {
    let route = vec![stop(10.0, 0.0, 30.0, vec![(0.0, content::Step::Run(true))]), stop(10.0, 10.0, 0.0, vec![]), stop(0.0, 10.0, 0.0, vec![])];
    let guard = patrol(route.clone(), 20.0, |_| {});
    assert_eq!((guard.next, guard.run), (1, true), "at the first place, bound for the second");
    let saved: MobSave = ron::from_str(&ron::to_string(&guard.snapshot()).unwrap()).unwrap();
    let mut back = Mob::new(content::creature_index("c1642"), Vec3::ZERO, 0.0);
    back.route = route;
    assert!(back.restore(&saved));
    assert_eq!((back.next, back.run), (1, true));
}

#[test]
fn a_stroll_is_one_to_three_walks_within_the_spawns_own_distance_and_zero_stands_still() {
    let far = Vec3::new(500.0, 0.0, 0.0);
    let (mut wolf, mut still) = (Mob::new(kind(), Vec3::ZERO, 0.0), Mob::new(kind(), Vec3::ZERO, 0.0));
    (wolf.wander, still.wander) = (12.0, 0.0);
    let (mut most_legs, mut farthest) = (0, 0.0f32);
    for _ in 0..(600.0 * 60.0) as usize {
        wolf.update(1.0 / 60.0, far, flat_ground, open);
        still.update(1.0 / 60.0, far, flat_ground, open);
        most_legs = most_legs.max(wolf.legs);
        farthest = farthest.max(flat(wolf.pos).length());
        assert!(still.state == State::Idle, "wander 0 never walks");
    }
    assert_eq!(most_legs, 3, "up to three walks in a row");
    assert!(farthest > 6.0 && farthest <= 12.5, "strolled {farthest} yards from home");
}

#[test]
fn a_friendly_creature_stands_still_cannot_be_hurt_and_can_be_talked_to() {
    let guard = content::creature_index("deputy_willem");
    let mut mob = Mob::new(guard, Vec3::new(2.0, 0.0, 0.0), 0.0);
    assert!(run(&mut mob, 5.0, Vec3::new(3.0, 0.0, 0.0)).is_empty());
    assert_eq!((mob.state, mob.pos), (State::Idle, Vec3::new(2.0, 0.0, 0.0)));
    assert!(!mob.struck_by(Vec3::new(2.0, 0.0, 3.0), 0.0, 3.5, 12.0) && mob.health == content::creature("deputy_willem").health);
    assert!(!mob.def.hostile && mob.def.title.is_some());
}

#[test]
fn a_neutral_creature_ignores_the_player_until_hit_then_fights_back() {
    let kind = content::creature_index("c299"); // Young Wolf
    let mut wolf = Mob::new(kind, Vec3::ZERO, 0.0);
    assert!(wolf.def.neutral && !wolf.def.hostile && wolf.def.attackable());
    assert!(run(&mut wolf, 3.0, Vec3::new(3.0, 0.0, 0.0)).is_empty() && !matches!(wolf.state, State::Chase | State::Melee(..)), "no aggro on sight");
    assert!(wolf.struck_by(wolf.pos + Vec3::Z * 2.0, 0.0, 3.5, 1.0), "it can be fought");
    let beside = wolf.pos + Vec3::X;
    let struck = run(&mut wolf, 3.0, beside);
    assert!(!struck.is_empty(), "it strikes back");
}

/// The quest marks bob up and down in their stand animation, and the "?" ones turn to face the viewer (their one bone
/// is a billboard locked to the vertical axis, 0x40), which the game has to do itself.
#[test]
fn the_quest_mark_models_bob_and_the_question_marks_face_the_viewer() {
    let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
    for path in MARKER_MODELS {
        let m2 = M2::parse(&mount.read(path).unwrap()).unwrap();
        let seq = m2.sequences.iter().position(|q| q.id == STAND).unwrap();
        let start = m2.sequences[seq].start as f32;
        let at = |t: f32| anim::bone_matrices(&m2, seq, start + t, t)[0];
        assert!(at(0.0).abs_diff_eq(at(1533.0), 1e-2) && !at(0.0).abs_diff_eq(at(800.0), 1e-3), "{path} does not bob");
        assert_eq!(m2.bones[0].flags & 0x40 != 0, path.contains("Question"), "{path}");
    }
}

#[test]
fn a_sneaking_player_is_noticed_only_from_close_by() {
    let range = aggro_range(content::creature("wolf").level, 1);
    let near_but_hidden = Vec3::new(range * SNEAK_AGGRO + 2.0, 0.0, 0.0);
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.hidden = true;
    assert!(wolf.alert(Vec3::new(range + 1.0, 0.0, 0.0), 1) == 0.0 && wolf.alert(near_but_hidden, 1) > 0.0 && wolf.alert(near_but_hidden, 1) < 1.0, "the eye opens as it comes close");
    assert!(run(&mut wolf, 1.0, near_but_hidden).is_empty() && wolf.state == State::Idle, "not noticed while sneaking");
    wolf.hidden = false;
    run(&mut wolf, 0.1, near_but_hidden);
    assert_eq!(wolf.state, State::Chase, "noticed once he stands up");
    // Sneaking right up to it works only so far.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.hidden = true;
    run(&mut wolf, 0.1, Vec3::new(range * SNEAK_AGGRO - 1.0, 0.0, 0.0));
    assert_eq!(wolf.state, State::Chase);
}

#[test]
fn a_wolf_follows_the_ground() {
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.update(0.016, Vec3::new(100.0, 0.0, 0.0), |x, _, _| Some(x * 0.5 + 2.0), open);
    assert_eq!(wolf.pos.y, 2.0);
}

/// Run `seconds` at 60 fps against the player `watch` describes; everything the mob did, in order.
fn acts_of(mob: &mut Mob, seconds: f32, watch: &Watch) -> Vec<Act> {
    (0..(seconds * 60.0) as usize).flat_map(|_| mob.step(1.0 / 60.0, watch, 0, &[], &[], flat_ground, open, &NavMesh::default())).collect()
}

fn casts(acts: &[Act]) -> Vec<(u32, Aim)> {
    acts.iter().filter_map(|a| if let Act::Cast(c) = a { Some((c.spell, c.aim)) } else { None }).collect()
}

#[test]
fn a_caster_stands_back_buffs_itself_and_casts_its_bolt_with_the_time_and_mana_it_takes() {
    // Defias Rogue Wizard: ranged mode at 35 yards, Frost Armor (12544) on itself, Frostbolt (13322) as its main spell, mana for many.
    let mut mage = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
    let mana = mage.mana;
    let player = Vec3::new(12.0, 0.0, 0.0);
    let acts = acts_of(&mut mage, 12.0, &Watch::at(player));
    let all = casts(&acts);
    assert!(all.contains(&(13322, Aim::Victim)), "it casts its bolt: {all:?}");
    // It stands where it is: twelve yards from the player is well inside its ranged distance.
    assert!(mage.pos.length() < 0.5, "it walked to {:?}", mage.pos);
    // A bolt takes its cast time: the begin comes well before the cast.
    let begin = acts.iter().position(|a| matches!(a, Act::Begin(13322))).expect("the cast begins");
    let gone = acts.iter().position(|a| matches!(a, Act::Cast(c) if c.spell == 13322)).expect("and goes off");
    assert!(gone > begin, "begin {begin}, cast {gone}");
    assert!(mage.mana < mana, "casting costs mana");
    // Frost Armor went on it, once (the script casts it while it is missing).
    assert!(all.contains(&(12544, Aim::Itself)) && mage.has_aura(12544), "{all:?}");
    assert_eq!(all.iter().filter(|c| c.0 == 12544).count(), 1);
}

#[test]
fn a_proximity_caster_backs_away_from_a_player_who_comes_close_and_a_wounded_one_runs_for_help() {
    let mut mage = Mob::new(content::creature_index("c474"), Vec3::new(20.0, 0.0, 0.0), 0.0);
    let near = Watch::at(Vec3::new(18.0, 0.0, 0.0));
    // Two yards from him, the mage steps back (ranged mode 2) before it casts its bolt.
    acts_of(&mut mage, 0.2, &near);
    assert!(matches!(mage.state, State::Kite(_)), "{:?} at {:?}", mage.state, mage.pos);
    acts_of(&mut mage, 1.0, &near);
    assert!(mage.pos.x > 21.0, "it backed away to {:?}", mage.pos);
    // Hurt to a tenth of its health, it runs (Flee at 15%) instead of fighting on.
    let mut seer = Mob::new(content::creature_index("c1397"), Vec3::ZERO, 0.0);
    let player = Watch::at(Vec3::new(10.0, 0.0, 0.0));
    acts_of(&mut seer, 0.5, &player);
    seer.health = seer.def.health * 0.1;
    acts_of(&mut seer, 0.2, &player);
    assert!(matches!(seer.state, State::Flee(..)), "{:?}", seer.state);
    // It runs away from the player, and after seven seconds it turns back on him.
    acts_of(&mut seer, 2.0, &player);
    assert!(seer.pos.x < -5.0, "ran to {:?}", seer.pos);
    acts_of(&mut seer, FLEE_TIME, &player);
    assert!(!matches!(seer.state, State::Flee(..)), "{:?}", seer.state);
}

#[test]
fn an_archer_shoots_from_range_and_a_creature_out_of_mana_goes_to_melee() {
    // Zanzil Hunter: ranged mode 1 at 24 yards, Shoot (a ranged attack: no stop to cast, then a short pause).
    let mut hunter = Mob::new(content::creature_index("c1489"), Vec3::ZERO, 0.0);
    let acts = acts_of(&mut hunter, 14.0, &Watch::at(Vec3::new(13.0, 0.0, 0.0)));
    let shots: Vec<_> = casts(&acts).into_iter().filter(|c| c.0 == 10277).collect();
    assert!(shots.len() >= 3, "it keeps shooting: {shots:?}");
    assert!(hunter.pos.length() < 0.5, "it stays back");
    // A caster whose main spell it cannot pay for walks up and fights close; a Frostmane Seer has 171 mana and its bolt takes 90.
    let mut mage = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
    mage.mana = 1.0;
    let player = Vec3::new(12.0, 0.0, 0.0);
    acts_of(&mut mage, 6.0, &Watch::at(player));
    assert!(flat(mage.pos - player).length() < MELEE_RANGE + 0.5, "out of mana it comes to melee: {:?}", mage.pos);
    let mut seer = Mob::new(content::creature_index("c1397"), Vec3::ZERO, 0.0);
    let bolts = casts(&acts_of(&mut seer, 14.0, &Watch::at(player))).iter().filter(|c| c.0 == 9532).count();
    assert_eq!(bolts, 1, "one bolt, and then its mana is gone");
    assert!(flat(seer.pos - player).length() < MELEE_RANGE + 0.5, "a Frostmane Seer then fights close: {:?}", seer.pos);
}

#[test]
fn a_dying_creature_casts_its_death_spell_and_a_summoner_calls_its_helpers() {
    let mut ogre = Mob::new(content::creature_index("c2252"), Vec3::ZERO, 0.0);
    acts_of(&mut ogre, 0.1, &Watch::at(Vec3::new(30.0, 0.0, 0.0)));
    ogre.hurt(ogre.def.health + 1.0);
    let acts = acts_of(&mut ogre, 0.1, &Watch::at(Vec3::new(2.0, 0.0, 0.0)));
    assert!(casts(&acts).iter().any(|c| c.0 == 9144), "Crushridge Death goes off: {:?}", casts(&acts));
    // Somebody in the data summons: its script asks for helpers once the fight is on.
    let defs = &content::get().creatures;
    let summoner = defs.iter().position(|d| d.attackable() && d.hostile && d.brain.events.iter().any(|e| e.on == brain::event::TIMER && e.acts.iter().any(|a| a.0 == brain::act::SUMMON))).expect("a summoner");
    let mut mob = Mob::new(summoner, Vec3::ZERO, 0.0);
    let acts = acts_of(&mut mob, 12.0, &Watch::at(Vec3::new(8.0, 0.0, 0.0)));
    assert!(acts.iter().any(|a| matches!(a, Act::Summon { .. })), "{} summoned nobody", mob.def.name);
}

#[test]
fn a_pull_calls_the_others_of_the_faction_and_a_helper_turns_on_the_player() {
    let mut leader = Mob::new(content::creature_index("c2240"), Vec3::ZERO, 0.0);
    let acts = acts_of(&mut leader, 0.5, &Watch::at(Vec3::new(8.0, 0.0, 0.0)));
    assert!(acts.iter().any(|a| matches!(a, Act::Assist(r) if *r > 0.0)), "it calls for help");
    let mut other = Mob::new(content::creature_index("c2240"), Vec3::new(5.0, 0.0, 5.0), 0.0);
    assert!(other.assist() && other.state == State::Chase);
    assert!(!other.assist(), "one that already fights is not called again");
}

#[test]
fn being_hit_holds_a_cast_back_instead_of_stopping_it() {
    let mut seer = Mob::new(content::creature_index("c474"), Vec3::ZERO, 0.0);
    let player = Watch::at(Vec3::new(12.0, 0.0, 0.0));
    acts_of(&mut seer, 0.7, &player);
    let State::Cast(spell, t, total, false) = seer.state else { panic!("{:?}", seer.state) };
    seer.hurt(1.0);
    let State::Cast(s2, t2, total2, false) = seer.state else { panic!("a hit does not break the cast: {:?}", seer.state) };
    assert!(s2 == spell && total2 == total && t2 < t, "pushed back from {t} to {t2}");
}

#[test]
fn an_archer_draws_the_weapon_it_holds_for_its_shot() {
    // Every shooter of the data that is a person with a ranged weapon in its outfit has the animation of that weapon.
    let defs = &content::get().creatures;
    let kinds: Vec<usize> = (0..defs.len()).filter(|&k| matches!(&defs[k].body, Body::Humanoid { outfit, .. } if outfit.iter().any(|&i| matches!(items::get(i).wear, items::Wear::Ranged)))).take(300).collect();
    assert!(!kinds.is_empty(), "somebody holds a bow or a gun");
    for k in kinds {
        let anim = Mob::new(k, Vec3::ZERO, 0.0).shot_anim();
        assert!([crate::animator::ATTACK_BOW, crate::animator::ATTACK_RIFLE, CAST_SPELL].contains(&anim), "{}: {anim}", defs[k].name);
    }
    let mob = Mob::new(content::creature_index("c1489"), Vec3::ZERO, 0.0);
    let (ready, cast) = mob.spell_anims(10277, &SpellAnims::new());
    assert_eq!(ready, cast);
    assert_eq!(mob.spell_anims(13322, &SpellAnims::new()), (READY_SPELL, CAST_SPELL), "a spell with no kit gets the usual stances");
}

#[test]
fn how_far_a_creature_notices_the_player_grows_a_yard_for_each_level_it_is_above_him() {
    assert_eq!(aggro_range(10, 10), 20.0);
    assert_eq!(aggro_range(15, 10), 25.0, "five levels above: five yards more");
    assert_eq!(aggro_range(5, 10), 15.0, "five levels below: five yards less");
    assert_eq!(aggro_range(40, 10), 45.0, "at most 45 yards, at 25 levels above and beyond");
    assert_eq!(aggro_range(1, 60), 5.0, "at least the reach of a blow");
    // A black wolf (level 3) notices a level-1 player from 22 yards, not from 23.
    let mut high = Mob::new(content::creature_index("black_wolf"), Vec3::ZERO, 0.0);
    let level = high.def.level;
    let at = |yards: f32| Watch { level: level - 2, ..Watch::at(Vec3::new(yards, 0.0, 0.0)) };
    high.step(0.1, &at(aggro_range(level, level - 2) + 1.0), 0, &[], &[], flat_ground, open, &NavMesh::default());
    assert_eq!(high.state, State::Idle);
    high.step(0.1, &at(aggro_range(level, level - 2) - 1.0), 0, &[], &[], flat_ground, open, &NavMesh::default());
    assert_eq!(high.state, State::Chase);
}

/// Fear sends it running from the player, a root holds its feet but not its teeth, a sleep ends with the first blow, a weakened creature takes more and deals less.
#[test]
fn fear_roots_sleep_and_weakness_change_what_a_creature_does() {
    let player = Vec3::new(5.0, 0.0, 0.0);
    // Terror: it runs to places round him (the original's flight is to random places round the one it flees from), then comes back to the fight when the time is up.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.afflict(Status::Fear(1.0));
    run(&mut wolf, 0.9, player);
    assert!(wolf.pos.length() > 4.0, "did not run: {:?}", wolf.pos);
    run(&mut wolf, 3.0, player);
    assert!(wolf.state.fights(), "{:?} {:?}", wolf.pos, wolf.state);
    // Rooted: it stays where it stands.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.afflict(Status::Root(2.0));
    run(&mut wolf, 1.0, player);
    assert_eq!(wolf.pos, Vec3::ZERO);
    run(&mut wolf, 2.0, player);
    assert!(wolf.pos.x > 0.5, "still held: {:?}", wolf.pos);
    // Asleep: it does nothing until it is struck; a sheep is a sheep until then.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.afflict(Status::Sheep(30.0));
    assert_ne!(wolf.look, wolf.kind);
    run(&mut wolf, 2.0, Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(wolf.pos, Vec3::ZERO);
    wolf.hurt(1.0);
    assert_eq!(wolf.look, wolf.kind);
    assert!(run(&mut wolf, 3.0, Vec3::new(2.0, 0.0, 0.0)).len() >= 1, "it did not wake");
    // Weakened: it takes a tenth more and strikes a bit less hard; banished, it cannot be hurt at all.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.afflict(Status::Weaken { dealt: 0.5, taken: 1.5, seconds: 10.0 });
    wolf.hurt(10.0);
    assert_eq!(wolf.health, def().health - 15.0);
    let strikes = run(&mut wolf, 3.0, Vec3::new(2.0, 0.0, 0.0));
    assert!(strikes.iter().all(|&d| (d - def().melee_damage * 0.5).abs() < 1e-3), "{strikes:?}");
    // Slowed in its blows: a quarter less speed means a third more time between swings, and it passes with the seconds.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let swing = wolf.swing();
    wolf.afflict(Status::Tempo(0.75, 5.0));
    assert!((wolf.swing() - swing / 0.75).abs() < 1e-4);
    wolf.afflicted(6.0);
    assert_eq!(wolf.swing(), swing);
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.afflict(Status::Banish(5.0));
    wolf.hurt(10.0);
    assert_eq!(wolf.health, def().health);
}

/// A creature that fights for the player follows him, goes for what is fighting him, strikes it, and is gone when its time is up.
#[test]
fn an_ally_follows_the_player_strikes_what_he_fights_and_goes_when_its_time_is_up() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let mut pet = Mob::new(kind(), Vec3::ZERO, 0.0);
    pet.make_ally(30, 10.0, 0, true, false);
    assert!(pet.is_ally() && pet.is_pet() && !pet.attackable());
    let step = |pet: &mut Mob, seconds: f32, focus: Option<Vec3>| -> Vec<Act> { (0..(seconds * 60.0) as usize).flat_map(|_| pet.step_ally(1.0 / 60.0, player, focus, false, flat_ground, open, &NavMesh::default())).collect() };
    // Nothing to fight: it runs to the player and stays close.
    step(&mut pet, 4.0, None);
    assert!((pet.pos - player).length() < 6.0, "{:?}", pet.pos);
    // Something to fight, 15 yards from him: it goes there and strikes, for more than its kind does (it is as strong as his level asks).
    let foe = Vec3::new(20.0, 0.0, 15.0);
    let acts = step(&mut pet, 5.0, Some(foe));
    let blows: Vec<f32> = acts.iter().filter_map(|a| if let Act::Strike(d) = a { Some(*d) } else { None }).collect();
    assert!((pet.pos - foe).length() < MELEE_RANGE + 0.5 && blows.len() >= 2, "{:?} {blows:?}", pet.pos);
    assert!(blows.iter().all(|&d| d > def().melee_damage), "{blows:?}");
    // The time is up: it is gone.
    step(&mut pet, 6.0, None);
    assert_eq!(pet.state, State::Unborn);
    // A pet hurt to death lies there; a revived one is up again.
    let mut pet = Mob::new(kind(), Vec3::ZERO, 0.0);
    pet.make_ally(1, 0.0, 0, true, false);
    pet.hurt(1e6);
    assert!(pet.is_dead() && pet.loot.is_none());
    step(&mut pet, 10.0, None);
    assert!(pet.is_dead(), "a pet lies until it is revived");
    pet.revive_ally();
    assert!(!pet.is_dead() && pet.health > 0.0);
}

/// A pet told to stay does not follow or chase; the stances pick foes as the pet bar says.
#[test]
fn a_pet_that_stays_keeps_its_place_and_the_stances_pick_their_foes() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let mut pet = Mob::new(kind(), Vec3::ZERO, 0.0);
    pet.make_ally(30, 0.0, 0, true, false);
    (0..600).for_each(|_| drop(pet.step_ally(1.0 / 60.0, player, Some(Vec3::new(0.0, 0.0, 15.0)), true, flat_ground, open, &NavMesh::default())));
    assert!(pet.pos.length() < 0.01, "{:?}", pet.pos);
    let pick = |stance| PetOrders { stance, ..Default::default() }.pick();
    assert_eq!(pick(Stance::Passive), None);
    assert_eq!(pick(Stance::Defensive).map(|p| p.1), Some(true));
    assert_eq!(pick(Stance::Aggressive).map(|p| p.1), Some(false));
}

/// A damage over time is listed with its spell's icon until it ends, the same spell cast again renews it instead of adding another, and it sets the creature on whoever sent it.
#[test]
fn a_spells_damage_over_time_is_listed_renewed_not_stacked_and_provokes() {
    let far = Vec3::new(100.0, 0.0, 0.0);
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    let full = wolf.health;
    let rend = Status::Over { amount: 3.0, period: 1.0, seconds: 5.0 };
    wolf.afflict_from(rend, "Rend", "Ability_Gouge", 0);
    assert_eq!(wolf.state, State::Chase);
    // (Left to itself, a wolf that cannot find the player goes home and heals: keep it still.)
    wolf.state = State::Idle;
    run(&mut wolf, 2.0, far);
    // Cast again: one bleed with a fresh five seconds, and one icon.
    wolf.afflict_from(rend, "Rend", "Ability_Gouge", 0);
    wolf.afflict_from(Status::Over { amount: 1.0, period: 1.0, seconds: 5.0 }, "Corruption", "Spell_Shadow_AbominationExplosion", 4);
    assert_eq!(wolf.shown_auras().0.len(), 2);
    wolf.state = State::Idle;
    run(&mut wolf, 6.0, far);
    // 2 ticks of 3, then 5 of 3 and 5 of 1.
    assert!((full - wolf.health - (6.0 + 15.0 + 5.0)).abs() < 1e-3, "{}", full - wolf.health);
    assert!(wolf.shown_auras().0.is_empty());
}

/// A warlock's imp casts its firebolt at what it fights, and the bolt hurts it.
#[test]
fn an_imp_casts_its_firebolt_at_its_foe() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    let foe = Vec3::new(20.0, 0.0, 14.0);
    let acts: Vec<Act> = (0..600).flat_map(|_| imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default())).collect();
    let bolts = acts.iter().filter(|a| matches!(a, Act::Spell(3110))).count();
    assert!(bolts >= 2, "{bolts} bolts, at {:?}", imp.pos);
    // And it plays its cast animation while the bolt goes off, not a swing.
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    let seen: Vec<u16> = (0..600).map(|_| { drop(imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default())); imp.animation(&Default::default()) }).collect();
    assert!(seen.contains(&CAST_SPELL) && !seen.contains(&ATTACK_UNARMED), "{:?}", seen.iter().collect::<std::collections::BTreeSet<_>>());
}

/// A pet casts nothing through a wall: it casts only once it has no wall between it and its foe.
#[test]
fn an_imp_does_not_cast_through_a_wall() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let foe = Vec3::new(20.0, 0.0, 14.0);
    // A wall across z = 8 that the imp cannot pass.
    let wall = |a: Vec3, b: Vec3| (a.z < 8.0) != (b.z < 8.0);
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    let acts: Vec<Act> = (0..600).flat_map(|_| imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, wall, &NavMesh::default())).collect();
    assert!(!acts.iter().any(|a| matches!(a, Act::Begin(_) | Act::Spell(_))), "cast through a wall, at {:?}", imp.pos);
}

/// An imp's firebolt is begun (the glow, the sound), goes off after its cast time, and is broken off when the foe is gone.
#[test]
fn an_imps_firebolt_is_begun_then_goes_off_after_its_cast_time() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let foe = Vec3::new(20.0, 0.0, 14.0);
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    let seconds = content::classic_spell(3110).unwrap().cast as f32 / 1000.0;
    let mut log = Vec::new();
    for n in 0..600 {
        for act in imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default()) {
            log.push((n as f32 / 60.0, matches!(act, Act::Begin(3110)), matches!(act, Act::Spell(3110))));
        }
    }
    let begun = log.iter().find(|a| a.1).expect("begun").0;
    let gone_off = log.iter().find(|a| a.2).expect("goes off").0;
    assert!((gone_off - begun - seconds).abs() < 0.1, "{begun} {gone_off} {seconds}");
    // The foe dies in the making: the cast stops.
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    let mut acts = Vec::new();
    for n in 0..120 {
        acts.extend(imp.step_ally(1.0 / 60.0, player, (n < 30).then_some(foe), false, flat_ground, open, &NavMesh::default()));
    }
    assert!(acts.iter().any(|a| matches!(a, Act::Begin(_))) && acts.iter().any(|a| matches!(a, Act::Stop)) && !acts.iter().any(|a| matches!(a, Act::Spell(_))), "{acts:?}");
}

/// A pet has the mana of the original's table for its kind and the level of its master, pays for its spells with it, fights close when it has too little, and
/// gets it back by its spirit every two seconds, but not within five seconds of spending some.
#[test]
fn an_imps_mana_is_the_originals_and_it_runs_dry_and_comes_back() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let foe = Vec3::new(20.0, 0.0, 14.0);
    let imp_at = |level: u32| {
        let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
        imp.make_ally(level, 0.0, 3110, true, false);
        imp
    };
    // The table of CMaNGOS (`pet_levelstats`): 48 mana at level 1 (spirit 23), 1898 at level 60.
    let max = |imp: &Mob| imp.ally.as_ref().unwrap().mana_max;
    assert_eq!((max(&imp_at(1)), max(&imp_at(60))), (48.0, 1898.0));
    // Four bolts of 10 leave 8: it cannot cast, so it goes up to its foe and strikes; five seconds after the last bolt (begun at 7.5 s) its mana comes back at the
    // next tick (14 s: spirit 23 / 5 + 17 = 21.6) and it casts a fifth (it goes off at 17 s).
    let mut imp = imp_at(1);
    let acts: Vec<Act> = (0..60 * 18).flat_map(|_| imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default())).collect();
    let bolts = acts.iter().filter(|a| matches!(a, Act::Spell(_))).count();
    assert_eq!(bolts, 5, "four bolts of 10 mana from 48, then none while it waits, then one more");
    assert!(acts.iter().any(|a| matches!(a, Act::Strike(_))), "dry, it fights close");
    // Out of mana from the start: no spell at all.
    let mut imp = imp_at(1);
    imp.ally.as_mut().unwrap().mana = 0.0;
    let acts: Vec<Act> = (0..60 * 2).flat_map(|_| imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default())).collect();
    assert!(!acts.iter().any(|a| matches!(a, Act::Spell(_) | Act::Begin(_))), "{acts:?}");
}

/// A pet has the health of the original's table for its kind and the level of its master, and gets the new level's health and mana (whole) when the master goes up.
#[test]
fn an_imp_has_the_originals_health_and_levels_up_with_its_master() {
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::ZERO, 0.0);
    imp.make_ally(1, 0.0, 3110, true, false);
    assert_eq!((imp.health, imp.ally_max(), imp.ally.as_ref().unwrap().mana_max), (140.0, 140.0, 48.0));
    imp.hurt(100.0);
    imp.ally.as_mut().unwrap().mana = 3.0;
    imp.set_ally_level(10);
    assert_eq!((imp.health, imp.ally_max(), imp.ally.as_ref().unwrap().mana, imp.ally_level()), (223.0, 223.0, 147.0, Some(10)));
    // A pet that lies dead has the new maximum, and is not healed by it; revived, it is half whole.
    imp.hurt(1e6);
    imp.set_ally_level(60);
    assert!(imp.is_dead() && imp.ally_max() == 769.0);
    imp.revive_ally();
    assert_eq!(imp.health, 384.5);
}

/// A pet out of phase is struck by nothing until it attacks: its spell or its blow ends the shift.
#[test]
fn a_pet_out_of_phase_is_in_phase_again_when_it_attacks() {
    let player = Vec3::new(20.0, 0.0, 0.0);
    let foe = Vec3::new(20.0, 0.0, 14.0);
    let mut imp = Mob::new(content::creature_index("c416"), Vec3::new(20.0, 0.0, 2.0), 0.0);
    imp.make_ally(10, 0.0, 3110, true, false);
    imp.ally.as_mut().unwrap().phased = true;
    assert!(imp.is_phased());
    drop((0..30).flat_map(|_| imp.step_ally(1.0 / 60.0, player, None, false, flat_ground, open, &NavMesh::default())).count());
    assert!(imp.is_phased(), "it stays out of phase while it does not attack");
    drop(imp.step_ally(1.0 / 60.0, player, Some(foe), false, flat_ground, open, &NavMesh::default()));
    assert!(!imp.is_phased(), "its spell begins");
}

/// The walled floor of the navigation tests, in render space: the wall runs across z = -10 from x = -30 to 30 (WoW's x = 10, y = -30 to 30).
fn behind_the_wall() -> (NavMesh, Vec3) {
    (crate::navmesh::tests::walled_floor(), Vec3::new(0.0, 0.0, -20.0))
}

#[test]
fn a_wolf_runs_home_round_a_wall_over_the_mesh_instead_of_through_it() {
    let (nav, home) = behind_the_wall();
    let wall = |a: Vec3, b: Vec3| (a.z + 10.0) * (b.z + 10.0) < 0.0 && a.x.abs().max(b.x.abs()) < 30.0;
    let mut wolf = Mob::new(kind(), home, 0.0);
    wolf.pos = Vec3::ZERO;
    wolf.state = State::Return(RETURN_WAIT);
    let player = Vec3::new(100.0, 0.0, 0.0);
    let mut last = wolf.pos;
    for _ in 0..(30.0 * 60.0) as usize {
        wolf.update_on(1.0 / 60.0, player, flat_ground, wall, &nav);
        assert!(!wall(last, wolf.pos), "through the wall: {last:?} to {:?}", wolf.pos);
        last = wolf.pos;
    }
    assert!(matches!(wolf.state, State::Idle | State::Wander(_)) && wolf.pos.z < -12.0, "stopped at {:?} ({:?})", wolf.pos, wolf.state);
}

#[test]
fn a_wolf_and_a_pet_chase_round_a_wall_over_the_mesh() {
    let (nav, player) = behind_the_wall();
    let wall = |a: Vec3, b: Vec3| (a.z + 10.0) * (b.z + 10.0) < 0.0 && a.x.abs().max(b.x.abs()) < 30.0;
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.state = State::Chase;
    let mut imp = Mob::new(kind(), Vec3::ZERO, 0.0);
    imp.make_ally(30, 0.0, 0, true, false);
    for _ in 0..(30.0 * 60.0) as usize {
        let before = (wolf.pos, imp.pos);
        wolf.update_on(1.0 / 60.0, player, flat_ground, wall, &nav);
        imp.step_ally(1.0 / 60.0, player, None, false, flat_ground, wall, &nav);
        assert!(!wall(before.0, wolf.pos) && !wall(before.1, imp.pos), "through the wall: {:?} {:?}", wolf.pos, imp.pos);
    }
    assert!((wolf.pos - player).length() < MELEE_RANGE + 1.0, "the wolf stopped at {:?}", wolf.pos);
    assert!((imp.pos - player).length() < 7.0, "the pet stopped at {:?}", imp.pos);
}




#[test]
fn a_wolf_on_a_platform_goes_to_its_edge_and_does_not_step_off_it() {
    // The platform is x = -20 to 0 (WoW), 5 yards high: in render space z = 0 to 20, y = 5. The player stands on the floor below its edge.
    let nav = crate::navmesh::tests::platform_on_floor();
    let mut wolf = Mob::new(kind(), Vec3::new(0.0, 5.0, 10.0), 0.0);
    wolf.state = State::Chase;
    let player = Vec3::new(0.0, 0.0, -6.0);
    let ground = |_: f32, z: f32, _: f32| Some(if (0.0..20.0).contains(&z) { 5.0 } else { 0.0 });
    for _ in 0..(20.0 * 60.0) as usize {
        wolf.update_on(1.0 / 60.0, player, ground, open, &nav);
        assert!(wolf.pos.y > 4.0, "left the platform: {:?}", wolf.pos);
    }
    // It went to the edge nearest the player.
    assert!(wolf.pos.z < 2.0, "{:?}", wolf.pos);
}

#[test]
fn a_wolf_that_stands_at_a_player_too_close_for_a_while_backs_off_and_one_in_anothers_place_moves_aside() {
    // The player stands almost on it: after the time the chase waits at its target, it walks back.
    let mut wolf = Mob::new(kind(), Vec3::ZERO, 0.0);
    wolf.state = State::Chase;
    let player = Vec3::new(1.0, 0.0, 0.0);
    let mut furthest = 0.0f32;
    for _ in 0..(25.0 * 60.0) as usize {
        wolf.update(1.0 / 60.0, player, flat_ground, open);
        furthest = furthest.max(wolf.pos.distance(player));
    }
    assert!(furthest > 1.5, "stood at {furthest} yards at most");
    // Another creature stands within a yard of it, fighting the same target: it goes round the target.
    let mut wolf = Mob::new(kind(), Vec3::new(2.0, 0.0, 0.0), 0.0);
    wolf.fan_out(Vec3::ZERO, &[Vec3::new(2.4, 0.0, 0.0)], &NavMesh::default(), &open);
    assert!(!wolf.course.is_empty() && wolf.chase.mode == Mode::Fanning, "{:?}", wolf.chase.mode);
    // And one that nobody stands beside stays.
    let mut alone = Mob::new(kind(), Vec3::new(2.0, 0.0, 0.0), 0.0);
    alone.fan_out(Vec3::ZERO, &[Vec3::new(9.0, 0.0, 0.0)], &NavMesh::default(), &open);
    assert!(alone.course.is_empty());
}
