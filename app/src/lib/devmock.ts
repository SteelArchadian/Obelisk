// Fake backend for `npm run dev` in a normal browser (no Tauri), so the UI can be clicked through.
// Never active in the real app.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const day = 86_400_000;
const now = Date.now();
const mod = (n: number, name: string, author: string, src: string) => ({
  fileName: `${name.toLowerCase().replace(/\W+/g, "_")}-1.${n}.0.jar`, name, version: `1.${n}.0`, authors: author,
  enabled: n % 7 !== 0, source: src, projectId: "", installedMs: now - n * day, modifiedMs: now - (n % 5) * day,
});
const mods = ["Create", "Jade", "JEI", "Sodium Extra", "Waystones", "Balm", "Farmer's Delight", "Supplementaries", "Iris", "Xaero's Minimap"]
  .map((n, i) => mod(i + 1, n, ["simibubi", "Snownee", "mezz", "FlashyReese", "BlayTheNinth"][i % 5], i % 3 ? "curseforge" : "modrinth"));
let trash: any[] = [{ id: "1-old", kind: "mods", fileName: "old_mod-2.0.jar", name: "Old Mod", version: "2.0", authors: "Someone", source: "modrinth", deletedMs: now - 2 * day }];

const TOML = `# Client settings
[client]
\t#Show the goggles overlay
\tshowGoggles = true
\t#How far away tooltips render
\t#Range: 1 ~ 64
\ttooltipDistance = 16
\t#Scale of the overlay
\t#Range: 0.5 ~ 2.0
\toverlayScale = 1.0
\t#Which corner the overlay sits in
\t#Allowed Values: TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT, BOTTOM_RIGHT
\tcorner = "TOP_LEFT"

[client.sound]
\t#Default: 0.5
\t#Range: 0.0 ~ 1.0
\tvolume = 0.8
\tdisabledSounds = ["create:whistle"]
`;

const JSON_CFG = JSON.stringify({ allowedCEM: "ALL", logModelCreationData: false, animationLODDistance: 20, modelsNamesDisabled: ["a", "b"], entityRenderModeOverrides: {}, nested: { scale: 1.5, on: true } }, null, 2);

export function installDevMock() {
  if ((window as any).__TAURI_INTERNALS__) return;
  mockWindows("main");
  mockIPC((cmd, args: any) => {
    switch (cmd) {
      case "get_settings": return { welcomed: true, plugins: { lightning: true }, theme: new URLSearchParams(location.search).get("theme") ?? undefined };
      case "list_instances": return [{ id: "Foundation", name: "Foundation", iconKey: "", minecraft: "1.21.1", loader: "NeoForge", loaderVersion: "21.1.252", secondsPlayed: 612000, lastLaunchMs: now }];
      case "list_content": return args.kind === "mods" ? mods : [];
      case "content_icons": return {};
      case "list_worlds": return [{ folder: "New World", lastPlayedMs: now, icon: null, datapacks: ["a.zip"] }];
      case "list_logs": return ["latest.log"];
      case "list_trash": return trash;
      case "restore_trash": trash = trash.filter((t) => t.id !== args.id); return {};
      case "delete_content": trash = [...trash, { id: `${Date.now()}`, kind: args.kind, fileName: args.fileName, name: args.name, version: args.version, authors: args.authors, source: args.source, deletedMs: Date.now() }]; return null;
      case "check_updates": return mods.map((m, i) => ({ kind: "mods", fileName: m.fileName, provider: m.source, projectId: String(i), current: "1", downloads: 0, env: ["client", "both", "server", ""][i % 4], update: i % 3 === 0 ? { id: "2", name: "new", fileName: "x.jar", blocked: false } : null }));
      case "mod_configs": return [{ path: "config/create-client.toml", size: 400 }, { path: "config/create-common.toml", size: 100 }, { path: "config/entity_model_features.json", size: 100 }, { path: "defaultconfigs/create-server.toml", size: 100 }];
      case "read_config": return args.path.endsWith(".json") ? JSON_CFG : TOML;
      case "config_default": return args.path.endsWith(".json") ? JSON_CFG : TOML;
      case "orphan_configs": return ["oldmod-common.toml", "spark"];
      case "backups": return {
        diskBytes: 7_340_032,
        pending: [{ kind: "changed", area: "configs", path: "config/create-common.toml", label: "create-common.toml" }],
        backups: [
          { id: "3", atMs: now - 3_600_000, kind: "safety", label: "Before Update all", files: 1114, bytes: 484_000_000, changes: [] },
          { id: "2", atMs: now - 86_400_000 / 2, kind: "auto", label: "", files: 1114, bytes: 484_000_000, changes: [
            { kind: "changed", area: "mods", path: "mods/create-6.0.2.jar", label: "Create 6.0.1 → 6.0.2" },
            { kind: "removed", area: "mods", path: "mods/jei-2.0.jar", label: "Just Enough Items 19.21" },
            { kind: "changed", area: "configs", path: "config/sodium-options.json", label: "sodium-options.json" },
          ] },
          { id: "1", atMs: now - 3 * 86_400_000, kind: "manual", label: "Before trying shaders", files: 1110, bytes: 480_000_000, changes: [] },
        ] };
      case "pack_trash": return [{ id: "1-Odyssey", instanceId: "Odyssey", name: "Odyssey", deletedMs: now - 2 * 86_400_000, bytes: 2_400_000_000 }];
      case "create_checkpoint": case "auto_backup": case "change_backup": case "safety_backup": case "restore_backup": case "revert_change": return null;
      case "share_overview": return { Foundation: 2 };
      case "share_status": return {
        shared: true, me: "me1",
        share: { packId: "p", name: "Foundation", autoApply: false, members: [
          { id: "me1", name: "You", lastSeenMs: now }, { id: "f1", name: "Alex", lastSeenMs: now - 60_000 }, { id: "f2", name: "Sam", lastSeenMs: now - 2 * 86_400_000 }] },
        local: [{ kind: "changed", area: "configs", path: "config/sodium-options.json", label: "sodium-options.json" }],
        incoming: [
          { id: "c1", author: "f1", authorName: "Alex", atMs: now - 3_600_000, label: "Added Farmer's Delight addons", changes: [
            { kind: "added", area: "mods", path: "mods/fd-addon.jar", label: "Farmer's Delight Addon 1.2" },
            { kind: "changed", area: "configs", path: "config/farmersdelight-common.toml", label: "farmersdelight-common.toml" }] },
          { id: "c2", author: "f2", authorName: "Sam", atMs: now - 1_800_000, label: "", changes: [{ kind: "removed", area: "mods", path: "mods/x.jar", label: "Xaero's Minimap 25.1" }] },
        ] };
      case "share_start": return "obelisk1.eyJwIjoiZGVtbyIsInMiOiJ4IiwibiI6IkZvdW5kYXRpb24iLCJhIjp7ImlkIjoiYWJjIn19";
      case "share_publish": return true;
      case "share_sync": return 0;
      case "share_apply": return { applied: 1, conflicts: [] };
      case "share_join": return "Foundation";
      case "share_skip": case "share_set_auto": case "share_stop": return null;
      case "mod_sides": return [
        { fileName: "sodium-neoforge-0.8.13+mc1.21.1.jar", name: "Sodium", ids: ["sodium"], side: "client", confidence: "likely", reasons: ["The mod only loads on the client (its loader dependency is client-side)."], neededBy: [] },
        { fileName: "create-1.1.0.jar", name: "Create", ids: ["create"], side: "both", confidence: "likely", reasons: ["Its download page lists it for client and server."], neededBy: ["Create: Steam 'n' Rails"] },
        { fileName: "iris-1.9.0.jar", name: "Iris", ids: ["iris"], side: "both", confidence: "guess", reasons: ["Kept on the server because Colorwheel needs it, though it looks client-only. Check the server starts."], neededBy: ["Colorwheel"] },
        { fileName: "obe-1.1.45.jar", name: "Optimized Block Entities", ids: ["obe"], side: "client", confidence: "guess", reasons: ["All its game patches (mixins) are client-only and most of its code is client code."], neededBy: [] },
        { fileName: "chunky-1.4.jar", name: "Chunky", ids: ["chunky"], side: "server", confidence: "likely", reasons: ["Its download page lists it as server-only."], neededBy: [] },
        { fileName: "jei-1.3.0.jar", name: "JEI", ids: ["jei"], side: "both", confidence: "sure", reasons: ["Has separate client and common entry points."], neededBy: [] },
      ];
      case "build_packs": return { serverDir: "C:/Users/you/Downloads/Foundation Server", serverZip: "C:/Users/you/Downloads/Foundation Server.zip", clientZip: "C:/Users/you/Downloads/Foundation Client.zip", notes: [] };
      case "profile_options": return { name: "Foundation", overrideMemory: false, maxMemMb: 8192, minMemMb: 1024, overrideJavaArgs: false, jvmArgs: "", overrideWindow: false, width: 1280, height: 720, maximized: false, overrideJavaLocation: false, javaPath: "" };
      case "java_installs": return [{ path: "C:/Prism/java/java-runtime-delta/bin/javaw.exe", version: "21.0.7", major: 21, vendor: "Microsoft" }, { path: "C:/Program Files/Java/jre1.8.0_481/bin/javaw.exe", version: "1.8.0_481", major: 8, vendor: "Oracle Corporation" }];
      case "prism_defaults": return { maxMemMb: 4096, jvmArgs: "", width: 1920, height: 1080, maximized: false };
      case "system_memory_mb": return 32768;
      case "prism_running": return false;
      case "read_report": return "[log text]";
      case "crash_reports": return [{ file: "crash-reports/crash-2026-10-16_15.53.34-client.txt", kind: "crash", timeMs: now - day }, { file: "hs_err_pid94448.log", kind: "jvm", timeMs: now - 3 * day }, { file: "logs/latest.log", kind: "log", timeMs: now - 3600_000 }];
      case "diagnose_crash": if (args.file === "logs/latest.log") return { ...{"file":"logs/latest.log","kind":"log","timeMs":1792549436630,"headline":"The last session ran, with some warnings","error":"","findings":[{"severity":"info","title":"The world lagged 1 time","detail":"The server fell behind 1 time; the worst was 2.7 s behind.","fix":"Fine right after joining a world. If it keeps happening while playing, use a profiler (Lightning or Spark) to see which mod is slow.","action":"","target":""},{"severity":"info","title":"A mod points at a file it doesn't ship","detail":"A mod lists an access-transformer file it doesn't include. NeoForge skips it; it only matters if that mod misbehaves. It points at pride.","fix":"","action":"","target":""},{"severity":"info","title":"1 error line worth a look","detail":"From Minecraft (1). Another 161 are known to be harmless.","fix":"These only matter if something in game doesn't work. Each group below says which mod it belongs to.","action":"","target":""},{"severity":"warning","title":"RoadWeaver held up loading for 38 seconds","detail":"RoadWeaver's GPU (OpenCL) path doesn't support this world's generation settings, so it plans roads on the CPU, which is much slower at world load.","fix":"If loading feels too slow, check RoadWeaver's settings or try the pack without it.","action":"page","target":"roadweaver-neoforge-2.3.2-1.21.1.jar"}],"suspects":[],"excerpt":["[20Oct2026 21:20:46.010] [main/ERROR] [net.neoforged.fml.loading.moddiscovery.ModFile/LOADING]: Access transformer file META-INF/accesstransformer.cfg provided by mod pride does not exist!"],"maxMemoryMb":0,"timeline":{"startupS":27.053,"worldS":127.921,"steps":[{"seconds":20.52,"phase":"world","who":"RoadWeaver","file":"roadweaver-neoforge-2.3.2-1.21.1.jar","note":"RoadWeaver's GPU (OpenCL) path doesn't support this world's generation settings, so it plans roads on the CPU, which is much slower at world load.","before":"[20Oct2026 21:22:27.511] [Server thread/INFO] [Ksyxis/MinecraftServerMixin/]: Ksyxis: Speeding up the world loading... Delete the mod, if it got stuck after this message. (net.minecraft.client.server.IntegratedServer pre","after":"[20Oct2026 21:22:48.031] [Server thread/INFO] [roadweaver/]: OpenCL 精采样不支持当前生成设置，使用 CPU: unsupported density node: com.ishland.c2me.opts.dfc.common.gen.jvm.CompiledDensityFunction"},{"seconds":15.665,"phase":"world","who":"RoadWeaver","file":"roadweaver-neoforge-2.3.2-1.21.1.jar","note":"RoadWeaver's GPU (OpenCL) path doesn't support this world's generation settings, so it plans roads on the CPU, which is much slower at world load.","before":"[20Oct2026 21:22:48.051] [RW-CoarseTile-1/INFO] [roadweaver/]: 粗高度采样使用 CPU","after":"[20Oct2026 21:23:03.716] [RW-Initial-1/INFO] [roadweaver/]: Presets loaded: 44 entries"},{"seconds":6.607,"phase":"world","who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","note":"Villager API re-validates every villager type and profession at startup and again at world load, even when no villager packs are installed.","before":"[20Oct2026 21:21:10.519] [modloading-worker-0/INFO] [villagerapi/]: Loaded villagerpacks_order.json (manual ordering: false)","after":"[20Oct2026 21:21:17.126] [modloading-worker-0/INFO] [villagerapi/]: Found 0 villager pack(s): 0 from villagerpacks/ and resourcepacks/, 0 from mods (manual ordering: false)"},{"seconds":6.592,"phase":"world","who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","note":"Villager API re-validates every villager type and profession at startup and again at world load, even when no villager packs are installed.","before":"[20Oct2026 21:21:17.132] [modloading-worker-0/INFO] [villagerapi/]: === Starting VillagerPack Validation ===","after":"[20Oct2026 21:21:23.724] [modloading-worker-0/INFO] [villagerapi/]: === Validation Complete ==="},{"seconds":6.254,"phase":"world","who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","note":"Villager API re-validates every villager type and profession at startup and again at world load, even when no villager packs are installed.","before":"[20Oct2026 21:22:13.224] [Server thread/INFO] [villagerapi/]: Validating POI types and professions...","after":"[20Oct2026 21:22:19.478] [Server thread/INFO] [villagerapi/]: Registered villager type mappings for 0 biome(s) from villagerpacks"},{"seconds":6.189,"phase":"world","who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","note":"Villager API re-validates every villager type and profession at startup and again at world load, even when no villager packs are installed.","before":"[20Oct2026 21:22:19.478] [Server thread/INFO] [villagerapi/]: Registered villager type mappings for 0 biome(s) from villagerpacks","after":"[20Oct2026 21:22:25.667] [Server thread/INFO] [villagerapi/]: Built villagerpack trades for 0 profession(s): []"},{"seconds":5.565,"phase":"world","who":"Moonlight Lib","file":"moonlight-1.21.1-3.7.1-neoforge.jar","note":"","before":"[20Oct2026 21:21:26.654] [Render thread/INFO] [net.minecraft.server.packs.repository.FolderRepositorySource/]: Found non-pack entry 'C:\\Users\\you\\AppData\\Roaming\\PrismLauncher\\instances\\Odyssey\\minecraft\\resourcepacks\\","after":"[20Oct2026 21:21:32.219] [Render thread/INFO] [Moonlight/]: Merging 2 images using mode MIN_AREA_NO_UPSCALE"},{"seconds":5.503,"phase":"world","who":"FancyMenu","file":"fancymenu_neoforge_3.9.14_MC_1.21.1.jar","note":"","before":"[20Oct2026 21:22:00.899] [Render thread/WARN] [net.minecraft.client.renderer.ShaderInstance/]: Shader cabb2374eda405e66b897908637abd1da5deac52 could not find uniform named subpixel_amount in the specified shader program.","after":"[20Oct2026 21:22:06.402] [Render thread/INFO] [de.keksuccino.fancymenu.customization.layer.ScreenCustomizationLayerHandler/]: [FANCYMENU] ScreenCustomizationLayer registered: select_world_screen"},{"seconds":5.19,"phase":"world","who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","note":"Villager API re-validates every villager type and profession at startup and again at world load, even when no villager packs are installed.","before":"[20Oct2026 21:21:05.329] [modloading-worker-0/WARN] [mixinsquared-canceller/]: Cancelled mixin neoforge.fun.qu_an.minecraft.asyncparticles.client.mixin.conditional.MixinParticles_LightCacheNoRefresh. Check debug logs for","after":"[20Oct2026 21:21:10.519] [modloading-worker-0/INFO] [villagerapi/]: Loaded villagerpacks_order.json (manual ordering: false)"},{"seconds":4.175,"phase":"world","who":"Speed Loader","file":"speedloader-1.21.1-0.1.5.jar","note":"","before":"[20Oct2026 21:21:52.848] [Worker-Main-6/INFO] [Boundless/]: Appearance cache: 237534 block states baked in 1031 ms","after":"[20Oct2026 21:21:57.023] [Render thread/INFO] [SpeedLoader/]: [timeline] overlayOff at 87813 ms"},{"seconds":3.025,"phase":"startup","who":"Sodium-GraphicsAdapterProbe","file":"","note":"","before":"[20Oct2026 21:20:33.171] [main/INFO] [cpw.mods.modlauncher.Launcher/MODLAUNCHER]: ModLauncher 11.0.5+main.901c6ea8 starting: java version 21.0.7 by Microsoft; OS Windows 11 arch amd64 version 10.0","after":"[20Oct2026 21:20:36.196] [main/INFO] [Sodium-GraphicsAdapterProbe/]: Searching for graphics cards..."},{"seconds":2.929,"phase":"startup","who":"ModuleLayerMigrator","file":"","note":"","before":"[20Oct2026 21:20:46.380] [main/INFO] [org.sinytra.connector.service.hacks.ModuleLayerMigrator/]: Successfully made module brigadier transformable","after":"[20Oct2026 21:20:49.309] [main/INFO] [mixin/]: Compatibility level set to JAVA_21"}],"byMod":[{"who":"RoadWeaver","file":"roadweaver-neoforge-2.3.2-1.21.1.jar","seconds":37.727},{"who":"VillagerAPI","file":"Villager-API-1.21.1-neoforge-1.26.9.6.jar","seconds":30.832},{"who":"Moonlight Lib","file":"moonlight-1.21.1-3.7.1-neoforge.jar","seconds":6.839},{"who":"Speed Loader","file":"speedloader-1.21.1-0.1.5.jar","seconds":5.9830003},{"who":"FancyMenu","file":"fancymenu_neoforge_3.9.14_MC_1.21.1.jar","seconds":5.503},{"who":"Minecraft","file":"","seconds":5.19},{"who":"Sodium-GraphicsAdapterProbe","file":"","seconds":3.025},{"who":"ModuleLayerMigrator","file":"","seconds":2.929}]},"errorGroups":[{"who":"Minecraft","file":"","count":1,"sample":"Access transformer file META-INF/accesstransformer.cfg provided by mod pride does not exist!"}],"noise":[{"label":"Recipe advancements that point at items from mods you don't have","count":137,"from":["Nomadic Tents"]},{"label":"Item Interactions settings for containers from mods you don't have","count":16,"from":["Item Interactions"]},{"label":"Controlify's FancyMenu add-on doesn't match this FancyMenu version (only that add-on is off)","count":2,"from":["FancyMenu","Controlify"]},{"label":"Controller mod checking for a Steam Deck","count":1,"from":["Controlify"]},{"label":"A mod checking for Fabric metadata on NeoForge","count":1,"from":["Mods Optimizer"]},{"label":"A mod's mixin warning (development leftover)","count":1,"from":["Minecraft"]},{"label":"A data map entry for something that doesn't exist","count":1,"from":["Minecraft"]},{"label":"Moonlight noting that Fabric mods run through Connector","count":1,"from":["Moonlight Lib"]},{"label":"A resource pack index that couldn't be read","count":1,"from":["LocalResourcePackIndex"]}]}, timeMs: now - 3600_000 };
        return args.file.startsWith("hs_err") ? { file: args.file, kind: "jvm", timeMs: now - 3 * day, headline: "The NVIDIA graphics driver crashed", error: "EXCEPTION_ACCESS_VIOLATION (0xc0000005)", maxMemoryMb: 6000,
          findings: [{ severity: "critical", title: "The NVIDIA graphics driver crashed", detail: "Java crashed inside the NVIDIA driver (nvoglv64.dll) while drawing the game.", fix: "Update your NVIDIA driver (a clean install helps).", action: "driver", target: "NVIDIA" }],
          suspects: [{ modId: "create", fileName: mods[1].fileName, reason: "This mod's code was running when the crash happened.", score: 10 }], excerpt: ["# Problematic frame:", "# C  [nvoglv64.dll+0xb80d80]"] }
        : { file: args.file, kind: "crash", timeMs: now - day, headline: "advancedcompassplus crashed the game", error: 'java.lang.NullPointerException: Cannot invoke "Font.width(String)" because "font" is null', maxMemoryMb: 6000,
          findings: [{ severity: "info", title: "advancedcompass is a Fabric mod running through Connector", detail: "It is loaded through Sinytra Connector.", fix: "Use a NeoForge version if there is one.", action: "", target: "" },
            { severity: "critical", title: "advancedcompassplus crashed the game", detail: "It tried to use something that wasn't there (a null value). The crash happened in this mod's own code.", fix: "Update advancedcompassplus. If it's already up to date, disable it to confirm, and report the crash to its author.", action: "disable", target: mods[0].fileName }],
          suspects: [{ modId: "advancedcompassplus", fileName: mods[0].fileName, reason: "The crash happened inside this mod's code.", score: 156 }, { modId: "advancedcompass", fileName: mods[1].fileName, reason: "This mod's code is in the crash's stack trace.", score: 100 }, { modId: "gnetum", fileName: "", reason: "This mod's code is in the crash's stack trace.", score: 60 }],
          excerpt: ["java.lang.NullPointerException: ...", "	at TRANSFORMER/advancedcompassplus@1.0.0/dev.chill.acplus.render.ThemeRenderer.drawCardinal(ThemeRenderer.java:156)"] };
      case "load_times": {
        const ph = (key: string, label: string, s: number, idle = false) => ({ key, label, ms: s * 1000, idle });
        const one = (ago: number, m: number) => ({ clickMs: now - ago, source: "obelisk", label: "", speedloader: true, phases: [
          ph("launcher", "Launcher and Prism", 2.4), ph("jvm", "Java starting", 1.1), ph("mods", "Loading mods", 30 * m), ph("resources", "Loading resources (to the menu)", 26 * m),
          ph("menu", "In the menu", 14, true), ph("join", "Joining world 1", 11 * m), ph("play", "Playing world 1", 1800, true), ph("quit", "Quitting", 3.2)] });
        return [one(3600_000, 1), one(86400_000, 1.2), one(2 * 86400_000, 1.1)];
      }
      case "lightning_runs": return { installed: true, runs: [{ id: "b", label: "flying", startedMs: now - 3600_000, html: "lightning/captures/lightning-b.html", phases: [], findings: [], contenders: [{ title: "Stutters: 235 frames took over 18.4 ms", size: "117 per minute, worst 195 ms" }, { title: "ixeris creates the most short-lived memory", size: "about 323 MB per second" }],
          whole: { seconds: 120, fps: 115.3, low1PctFps: 41.2, frameP99: 25.99, tickP50: 9, heapMaxMB: 6000, liveSetMBmax: 5300, topModsCpu: [{ mod: "ixeris", sharePct: 41.75 }, { mod: "boundless", sharePct: 25.68 }, { mod: "iris", sharePct: 19.96 }, { mod: "accessories", sharePct: 9.3 }, { mod: "jade", sharePct: 2.1 }] } },
          { id: "a", label: "", startedMs: now - 2 * day, html: "", phases: [], findings: [], contenders: [], whole: { seconds: 60, fps: 98, low1PctFps: 30, topModsCpu: [{ mod: "boundless", sharePct: 30 }] } }] };
      case "get_pins": return [];
      case "has_cf_key": return true;
      case "list_accounts": return [];
      case "mr_project": return { id: "LNytGWDc", slug: "create", title: "Create", description: "Aesthetic technology", project_type: "mod", body: "## Create Rotational power.", gallery: [], loaders: ["neoforge"], issues_url: "github.com/Creators-of-Create/Create/issues", source_url: "https://github.com/Creators-of-Create/Create", downloads: 5_000_000, followers: 1, icon_url: "", categories: ["technology"], game_versions: ["1.21.1"], versions: [], date_modified: new Date(now).toISOString(), published: new Date(now).toISOString(), team: "t" };
      case "mr_versions": case "mr_members": return [];
      case "mr_account": return { id: "u1", username: "DemoUser", avatar_url: "" };
      case "mr_follows": return [];
      case "mr_follow": case "mr_notifications_read": return null;
      case "mr_notifications": return [{ id: "n1", title: "Sodium has been updated", text: "Version 0.6.13 is out", link: "/project/sodium", read: false, created: new Date(now - 3600_000).toISOString() }, { id: "n2", title: "You were invited to a team", text: "", link: "/user/x", read: true, created: new Date(now - 3 * day).toISOString() }];
      case "mr_dependencies": return { projects: [{ id: "P7dR8mSH", title: "Fabric API", icon_url: "", project_type: "mod", downloads: 9000000 }], versions: [] };
      case "cf_relations": return { data: [
        { id: 1, name: "Create: Steam 'n' Rails", authorName: "Layers", logoUrl: "", type: "RequiredDependency", categoryClass: { name: "Mods" }, downloads: 4200000 },
        { id: 2, name: "All the Mods 10", authorName: "ATM", logoUrl: "", type: "Include", categoryClass: { name: "Modpacks" }, downloads: 9000000 },
        { id: 3, name: "Create Deco", authorName: "Rabbit", logoUrl: "", type: "OptionalDependency", categoryClass: { name: "Mods" }, downloads: 800000 }],
        pagination: { totalCount: 3 } };
      case "mr_installed": return [];
      default: return null;
    }
  });
}
