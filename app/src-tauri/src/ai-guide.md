# Obelisk for AI assistants

Obelisk manages the user's Minecraft modpacks (Prism instances) and runs a test bench of throwaway copies for you. Games
run outside your sandbox at normal speed, muted, with the window off-screen. The user's own pack ("play") is never
launched: you can read it, and change its mods (each change takes a safety backup first). Every test runs in a copy.

## Rules
- Make a test copy with copy_pack {from: "play", to: "<name>"}; list copies with copies.
- Launches share one bench lock and queue. Measured runs (perf, fps, stress, dyno) wait until the user has been away
  for 10 minutes and end "invalid" if they come back. Hold <=120 s (up to 600 s only for a non-perf "interaction" run);
  a run is killed as "overtime" at 240 s + hold.
- Mod changes (mod_set, mod_add, mod_install, mod_update) wait for that instance's game to close.
- Use wait (it blocks) rather than polling status. End runs with kill when no save is needed.

## Simulations: script a situation
game_test {copy, owner, purpose, preset, steps} launches, waits for the world, runs the steps, quits normally (saving)
and returns load stages, per-phase resources, each step's result, log errors by mod and the profiler summary.
background: true returns the job at once; read it with game {op: result}.

- preset "scene": a 64x64 flat grass world, time (midnight, with night vision: day-bright, undead never burn), weather
  and mob spawning frozen, creative; steps start once the terrain is drawn. About as fast as
  the pack can load.
- preset "basic": visit four structures. "stress": chests, mobs, hop, flyover. "dyno": Block Dyno's heavy plan.
- Your own world: test_world {layers [{block,height}], terrain flat|normal, border, platform, spawn, game_mode,
  setup [commands], events [{at,every,command}]}, or world / new_world.

A scene, or any run with shot, punch or record steps, turns off for that run the mods that move the camera or slow
the window on their own (AFK and respawn cinematics, teleport fly-ins, Dynamic FPS) and hides chat.

Steps, in order, as granular as you need:
- {cmd: "<any command>"} runs as the player (op level). {tp: [x,y,z,(yaw,pitch)]}. {wait: seconds} (fractions work).
- {summon: {mob: "zombie", distance: 3, ai: false, knockback: false, nbt: "..."}}: in front of the player, who faces
  it; no AI and no knockback by default, so it stays in frame.
- {punch: {times: 5, every: 1.0, shots: [0.5, 0.75, 1.75], label: "z"}}: real player punches (swing, attack,
  cooldown); screenshots at those offsets after each punch. Each reports what was hit and its health.
- {shot: "label"}: a screenshot now. Shots land in <copy>/screenshots/obt-<run>-<label>.png; the same labels in
  two runs give comparable pictures.
- {record: {seconds: 10, label: "x"}} (max 30): watches every frame and reports what you would otherwise have to stare
  at: flicker (spots that change for one frame and snap back), sudden colour or brightness jumps, stutters, and how
  often each part of the screen really updates (an animation stepping at 12/s while the game draws 60). Frames
  before/during/after the first events are saved as PNGs (obr-*.png); per-frame numbers in .obelisk/record-*.csv.
  Hold the camera still (and use a scene) so only the thing under test moves, or give during: [steps] (fly, tp,
  cmd...) to record while those run, for artifacts that only show in motion; flicker is still found while moving
  (motion never snaps back), update rates then mean less.
- {sample: seconds}: fps, 1% low, mspt over that time. {query: true}: fps, mspt, heap, chunks, entities, player.
- {settle: max_seconds}: until the player has stayed put for 2 s (scene does this first).
- {wait_chunks: seconds}, {fly: {path, seconds}}, {locate: [structures]}, {visit}, {hop}, {flyover}, {mobs}, {chests}.
- {assert: {field, min|max|equals}} checks the previous step's result.
While a game runs: game {op: query|cmd|scenario|quit|stop}; heap for histograms, dumps and thread stacks.

## Find the mod behind a behaviour (bisect, "Thanos")
bisect {copy, action: start, run: <game_test args>, keep: [file prefixes never disabled]}: each round runs your
scenario with half the suspects disabled for that run only (mods that others require stay on). Look at the result and
shots, then bisect {copy, action: answer, present: true|false}. About log2(mods) rounds to one culprit. Also retry,
status, stop.

## Tests, profiling and packs
- launch: full control of one run (jvm properties, mods on/off for the run, configs, options, Java 21/25, GC, JFR,
  hooks, camera, perf). gate: full-pack gate with a candidate jar.
- lightning, timeline, screenshots, reports, diagnose (crash cause and the mods at fault), log (grep, tail).
- mods, updates, mod_update, mod_install (Modrinth or CurseForge, with dependencies), mod_add, mod_set.
- release_submit / release_status / release_promote / release_pack: ship a mod build to the packs, gated, with
  automatic rollback on a crash naming it.
- app_commands / app: run anything the Obelisk window can do. Ask the user before anything big or hard to undo.
- errors: what Obelisk itself hit.
