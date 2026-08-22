// Per-model icon specs.
//
// Each entry answers one question: *what does this thing look like on a pedalboard?* Line 6's model
// names are puns on the hardware they model, so the shape/colour/knob-count cue is exactly the cue
// a player already has — a gold three-knob box, a big silver fuzz, a green four-switch delay. That
// is all we encode: silhouette, finish, controls. No logos, no lettering, no artwork.
//
// Anything not listed here still gets an icon — `spec.js` falls back to the effect family (chorus,
// spring reverb, tape echo, …) and then to the category. Adding a model is one line.

import { C, PANEL, CLOTH } from "./palette.js";

const box = (body, knobs, o = {}) => ({ shape: "stomp", body, knobs, ...o });
const wide = (body, knobs, o = {}) => ({ shape: "stompWide", body, knobs, ...o });
const mini = (body, knobs, o = {}) => ({ shape: "stompNarrow", body, knobs, ...o });
const rack = (body, knobs, o = {}) => ({ shape: "rack", body, knobs, ...o });
const wah = (body, o = {}) => ({ shape: "wah", body, ...o });
const reel = (body, knobs, o = {}) => ({ shape: "reel", body, knobs, ...o });
const util = (jacks, o = {}) => ({ shape: "util", jacks, ...o });

export const MODELS = {
  // ---- Distortion ------------------------------------------------------
  HD2_DistKinkyBoost: mini(C.navy, 1, { led: "#7fd4ff" }),
  HD2_DistDerangedMaster: { shape: "wedge", body: C.sand, knobs: 1 },
  HD2_DistMinotaur: box(C.gold, 3, { led: "#ffd166" }),
  HD2_DistTeemah: box(C.aqua, 4, { layout: "grid" }),
  HD2_DistHeirApparent: box(C.cream, 3),
  HD2_DistToneSovereign: wide(C.cream, 6, { layout: "grid", sw: 2 }),
  HD2_DistAlpacaRouge: box(C.red, 2),
  HD2_DistCompulsiveDrive: box(C.white, 4, { layout: "grid" }),
  HD2_DistDhyanaDrive: box(C.indigo, 4, { layout: "grid" }),
  HD2_DistHorizonDrive: box(C.black, 4, { layout: "grid", led: "#ff8a3d" }),
  HD2_DistValveDriver: wide(C.ivory, 4),
  HD2_DistTopSecretOD: box(C.steel, 2, { plate: C.yellow }),
  HD2_DistPrizeDrive: box(C.teal, 3),
  HD2_DistScream808: box(C.green, 3),
  HD2_DistPillars: box(C.sky, 4, { layout: "grid" }),
  HD2_DistHedgehogD9: box(C.blue, 3),
  HD2_DistStuporOD: box(C.yellow, 3, { plate: "#c9ae3a" }),
  HD2_DistDeezOneVintage: box(C.orange, 3, { plate: "#c46520" }),
  HD2_DistDeezOneMod: box(C.orange, 4, { plate: "#c46520", layout: "grid" }),
  HD2_DistRatatouilleDist: box(C.charcoal, 3, { led: "#ff5c5c" }),
  HD2_DistVerminDist: box(C.black, 3, { plate: "#33373d" }),
  HD2_DistVitalDist: box(C.crimson, 4, { layout: "grid" }),
  HD2_DistVitalBoost: mini(C.crimson, 1),
  HD2_DistKWB: box(C.charcoal, 3),
  HD2_DistLegendaryDrive: box(C.oxblood, 3),
  HD2_DistSwedishChainsaw: box(C.steel, 4, { plate: "#6b7079", layout: "grid" }),
  HD2_DistArbitratorFuzz: { shape: "round", body: C.navy },
  HD2_DistPocketFuzz: mini(C.red, 1),
  HD2_DistRamsHead: wide(C.silver, 3, { plate: "#9096a0" }),
  HD2_DistTriangleFuzz: wide(C.white, 3, { layout: "arc" }),
  HD2_DistDarkDoveFuzz: wide(C.black, 3),
  HD2_DistBallisticFuzz: box(C.graphite, 3),
  HD2_DistIndustrialFuzz: box(C.steel, 4, { plate: "#6b7079", layout: "grid" }),
  HD2_DistTycoctaviaFuzz: { shape: "wedge", body: C.orange, knobs: 2 },
  HD2_DistWringerFuzz: box(C.magenta, 3),
  HD2_DistThrifterFuzz: box(C.tan, 3),
  HD2_DistXenomorphFuzz: box(C.forest, 4, { layout: "grid", led: "#7cffb0" }),
  HD2_DistMegaphone: box(C.charcoal, 4, { layout: "grid", mark: "window" }),
  HD2_DistBitcrusher: box(C.graphite, 4, { layout: "grid", mark: "window", led: "#7fd4ff" }),
  HD2_DistAmpegScramblerOD: box(C.navy, 3, { plate: C.silver }),
  HD2_DistZeroAmpBassDI: wide(C.black, 6, { layout: "grid" }),
  HD2_DistRegalBassDI: wide(C.espresso, 5, { layout: "grid" }),
  HD2_DistObsidian7000: wide(C.black, 6, { layout: "grid", led: "#ff8a3d" }),
  HD2_DistClawthornDrive: box(C.forest, 4, { layout: "grid" }),
  // DM4 legacy drives — the same hardware, one generation earlier.
  HD2_DM4TubeDrive: wide(C.ivory, 4),
  HD2_DM4Screamer: box(C.green, 3),
  HD2_DM4Overdrive: box(C.steel, 2, { plate: C.yellow }),
  HD2_DM4ClassicDistortion: box(C.orange, 3, { plate: "#c46520" }),
  HD2_DM4HeavyDistortion: box(C.black, 4, { plate: C.gold, layout: "grid" }),
  HD2_DM4ColorDrive: box(C.amber, 3),
  HD2_DM4BuzzSaw: box(C.black, 3, { plate: "#33373d" }),
  HD2_DM4FacialFuzz: { shape: "round", body: C.crimson },
  HD2_DM4JumboFuzz: wide(C.charcoal, 3),
  HD2_DM4FuzzPi: wide(C.white, 3, { layout: "arc" }),
  HD2_DM4JetFuzz: wide(C.silver, 4),
  HD2_DM4Line6Drive: box(C.graphite, 3, { led: "#ff5c5c" }),
  HD2_DM4Line6Distortion: box(C.graphite, 4, { layout: "grid", led: "#ff5c5c" }),
  HD2_DM4SubOctFuzz: box(C.purple, 3),
  HD2_DM4OctaveFuzz: { shape: "wedge", body: C.orange, knobs: 2 },
  Line6BronzeMaster: box(C.tan, 3, { led: "#ffd166" }),
  KillerZ: box(C.oxblood, 4, { layout: "grid" }),

  // ---- Dynamics --------------------------------------------------------
  HD2_CompressorDeluxeComp: box(C.sky, 4, { layout: "grid" }),
  HD2_CompressorRedSqueeze: box(C.red, 2),
  HD2_CompressorKinkyComp: mini(C.aqua, 2),
  HD2_CompressorOptoComp: box(C.navy, 3, { plate: C.silver }),
  HD2_CompressorRochesterComp: box(C.navy, 4, { plate: C.silver, layout: "grid" }),
  HD2_CompressorLAStudioComp: rack(C.steel, 2, { vu: true }),
  HD2_Compressor3BandComp: rack(C.charcoal, 6, { layout: "grid" }),
  HD2_CompressorTransientShaper: rack(C.graphite, 2),
  HD2_CompressorAutoSwell: box(C.silver, 3),
  HD2_GateNoiseGate: box(C.charcoal, 2, { led: "#7cffb0" }),
  HD2_GateHardGate: box(C.black, 3, { led: "#7cffb0" }),
  HD2_GateHorizonGate: box(C.black, 3, { led: "#ff8a3d" }),
  VIC_FeedbackSim: box(C.magenta, 3),
  HD2_DM4TubeComp: rack(C.steel, 2, { vu: true }),
  HD2_DM4RedComp: box(C.red, 2),
  HD2_DM4BlueComp: box(C.blue, 2),
  HD2_DM4BlueCompTreb: box(C.blue, 3),
  HD2_DM4VettaComp: box(C.graphite, 2, { led: "#ff5c5c" }),
  HD2_DM4VettaJuice: box(C.graphite, 3, { led: "#ff5c5c" }),
  HD2_DM4BoostComp: mini(C.lime, 2),

  // ---- EQ --------------------------------------------------------------
  HD2_EQSimple3Band: rack(C.charcoal, 3),
  HD2_EQLowCutHighCut: rack(C.charcoal, 2),
  HD2_EQLowShelfHighShelf: rack(C.charcoal, 4),
  HD2_EQParametric: rack(C.graphite, 6, { layout: "grid" }),
  HD2_EQSimpleTilt: rack(C.charcoal, 2),
  HD2_EQGraphic10Band: { shape: "rack", body: C.black, sliders: 10 },
  HD2_CaliQ: { shape: "rack", body: C.black, sliders: 5 },
  L6SPB_AcousGtrSim: box(C.tweed, 3),

  // ---- Modulation ------------------------------------------------------
  HD2_TremoloOpticalTrem: box(C.blue, 3),
  HD2_Tremolo60sBiasTrem: box(C.espresso, 3),
  HD2_TremoloTremolo: box(C.green, 3),
  HD2_TremoloHarmonic: box(C.brown, 4, { layout: "grid" }),
  HD2_TremoloPattern: box(C.charcoal, 4, { layout: "grid", mark: "window" }),
  HD2_PhaserScriptModPhase: box(C.orange, 1),
  HD2_PhaserPebblePhaser: box(C.steel, 1),
  HD2_PhaserUbiquitousVibe: wide(C.silver, 2, { plate: "#9096a0" }),
  VIC_FlexoVibe: wide(C.navy, 3),
  HD2_PhaserDeluxePhaser: box(C.violet, 4, { layout: "grid" }),
  HD2_FlangerGrayFlanger: wide(C.steel, 4),
  HD2_FlangerHarmonicFlanger: wide(C.silver, 4),
  HD2_FlangerCourtesanFlange: wide(C.chrome, 4),
  HD2_FlangerDynamixFlanger: box(C.sky, 4, { layout: "grid" }),
  HD2_Chorus: box(C.aqua, 3),
  HD2_Chorus70sChorus: wide(C.steel, 2, { sw: 2 }),
  HD2_ChorusPlastiChorus: box(C.silver, 3),
  HD2_ChorusAmpegLiquifier: box(C.navy, 3, { plate: C.silver }),
  HD2_Chorus4Voice: rack(C.graphite, 4),
  HD2_ChorusTrinityChorus: rack(C.charcoal, 5),
  HD2_VibratoBubbleVibrato: box(C.blue, 3),
  HD2_RetroReel: reel(C.tan, 3),
  HD2_DelayDoubleDouble: box(C.teal, 4, { layout: "grid" }),
  L6SPB_PolyChorus: box(C.aqua, 4, { layout: "grid" }),
  HD2_RingModulatorAMRingMod: box(C.charcoal, 3, { mark: "window" }),
  HD2_RingModulatorPitchRingMod: box(C.charcoal, 4, { mark: "window", layout: "grid" }),
  HD2_RotaryVibeRotary: { shape: "rotary", body: C.espresso },
  HD2_Rotary122Rotary: { shape: "rotary", body: C.brown },
  HD2_Rotary145Rotary: { shape: "rotary", body: C.espresso },
  HD2_Rotary3Rotor: { shape: "rotary", body: C.black },
  HD2_MM4RotaryDrum: { shape: "rotary", body: C.brown },
  HD2_MM4RotaryDrumHorn: { shape: "rotary", body: C.espresso },
  HD2_MM4PatternTrem: box(C.charcoal, 4, { layout: "grid", mark: "window" }),
  HD2_MM4Panner: box(C.teal, 3),
  HD2_MM4BiasTremolo: box(C.espresso, 3),
  HD2_MM4OptoTremolo: box(C.blue, 3),
  HD2_MM4ScriptPhase: box(C.orange, 1),
  HD2_MM4PannedPhaser: box(C.violet, 3),
  HD2_MM4BarberpolePhaser: box(C.magenta, 3),
  HD2_MM4DualPhaser: box(C.violet, 4, { layout: "grid" }),
  HD2_MM4UVibe: wide(C.silver, 2, { plate: "#9096a0" }),
  HD2_MM4Phaser: box(C.orange, 2),
  HD2_MM4PitchVibrato: box(C.blue, 3),
  HD2_MM4Dimension: wide(C.chrome, 0, { sw: 4 }),
  HD2_MM4AnalogChorus: box(C.aqua, 3),
  HD2_MM4TriChorus: rack(C.charcoal, 5),
  HD2_MM4AnalogFlanger: wide(C.steel, 4),
  HD2_MM4JetFlanger: wide(C.silver, 4),
  HD2_M13ACFlanger: wide(C.chrome, 4),
  HD2_M1380AFlanger: wide(C.charcoal, 4),
  HD2_MM4FrequencyShifter: box(C.indigo, 4, { layout: "grid", mark: "window" }),
  HD2_MM4RingModulator: box(C.charcoal, 3, { mark: "window" }),
  TapeEater: reel(C.brown, 4),
  Warble_Matic: box(C.mint, 4, { layout: "grid" }),
  SampleAndHold: box(C.indigo, 3, { mark: "window" }),
  Sweeper: box(C.violet, 4, { layout: "grid" }),

  // ---- Delay -----------------------------------------------------------
  HD2_DelaySimpleDelay: box(C.graphite, 3, { mark: "window" }),
  HD2_DelayModChorusEcho: box(C.teal, 4, { layout: "grid", mark: "window" }),
  HD2_DelaySweepEcho: box(C.indigo, 4, { layout: "grid", mark: "window" }),
  HD2_DelayDuckedDelay: box(C.graphite, 4, { layout: "grid", mark: "window" }),
  HD2_DelayReverseDelay: box(C.purple, 3, { mark: "window" }),
  HD2_DelayVintageDigital: box(C.silver, 4, { layout: "grid", mark: "window" }),
  HD2_DelayVintageDigitalV2: box(C.silver, 4, { layout: "grid", mark: "window" }),
  HD2_DelaySwellVintageDigital: box(C.silver, 4, { layout: "grid", mark: "window" }),
  HD2_DelayPitch: box(C.violet, 4, { layout: "grid", mark: "window" }),
  HD2_DelayTransistorTape: reel(C.charcoal, 3),
  HD2_DelayCosmosEcho: reel(C.forest, 4),
  HD2_DelayBucketBrigade: box(C.blue, 3),
  HD2_DelayAdriaticDelay: box(C.sky, 3),
  HD2_DelaySwellAdriatic: box(C.sky, 4, { layout: "grid" }),
  HD2_DelayElephantMan: wide(C.chrome, 5, { layout: "grid" }),
  HD2_DelayMultiPass: box(C.graphite, 6, { layout: "grid", mark: "window" }),
  HD2_DelayHeliosphere: box(C.indigo, 5, { layout: "grid", mark: "window" }),
  L6SPB_InfSustain: box(C.mint, 3, { mark: "window" }),
  VIC_DelayPolySustain: box(C.mint, 3, { mark: "window" }),
  Victoria_ShufflingDelay: box(C.magenta, 4, { layout: "grid", mark: "window" }),
  VIC_DelayGlitch: box(C.magenta, 4, { layout: "grid", mark: "window" }),
  Victoria_EuclideanDelay: box(C.aqua, 5, { layout: "grid", mark: "window" }),
  VIC_DelayStutterEdit: box(C.pink, 5, { layout: "grid", mark: "window" }),
  VIC_DelayRatchet: box(C.pink, 4, { layout: "grid", mark: "window" }),
  HD2_DelayADT: box(C.steel, 3, { mark: "window" }),
  HD2_DelayCrissCross: box(C.teal, 5, { layout: "grid", mark: "window" }),
  HD2_DelayDualDelay: box(C.graphite, 6, { layout: "grid", mark: "window" }),
  HD2_DelayMultitap4: box(C.graphite, 4, { layout: "grid", mark: "bars" }),
  HD2_DelayMultitap6: box(C.graphite, 6, { layout: "grid", mark: "bars" }),
  HD2_DelayPingPong: box(C.teal, 4, { layout: "grid", mark: "window" }),
  HD2_DelayHarmonyDelay: box(C.violet, 5, { layout: "grid", mark: "window" }),
  L6BubbleEcho: box(C.aqua, 4, { layout: "grid", mark: "window" }),
  L6PhazeEko: box(C.violet, 4, { layout: "grid", mark: "window" }),

  // ---- Reverb ----------------------------------------------------------
  HD2_Reverb63Spring: { shape: "spring", body: C.espresso },
  HD2_ReverbSpring: { shape: "spring", body: C.charcoal },
  HD2_ReverbHxSpring: { shape: "spring", body: C.oxblood },
  HD2_ReverbDoubleTank: { shape: "spring", body: C.navy, tanks: 2 },
  HD2_ReverbPlate: rack(C.steel, 3, { glyph: "plate" }),
  VIC_DynPlate: rack(C.steel, 4, { glyph: "plate" }),
  HD2_ReverbRoom: rack(C.charcoal, 3, { glyph: "room" }),
  VIC_ReverbDynRoom: rack(C.charcoal, 4, { glyph: "room" }),
  HD2_ReverbChamber: rack(C.graphite, 3, { glyph: "room" }),
  HD2_ReverbTile: rack(C.silver, 3, { glyph: "room" }),
  HD2_ReverbHall: rack(C.navy, 3, { glyph: "arch" }),
  VIC_ReverbRotating: rack(C.navy, 4, { glyph: "arch" }),
  HD2_ReverbCave: rack(C.black, 3, { glyph: "cave" }),
  VIC_ReverbDynAmbience: rack(C.teal, 4, { glyph: "room" }),
  VIC_ReverbDynBloom: rack(C.violet, 4, { glyph: "arch" }),
  VIC_ReverbShimmer: rack(C.aqua, 4, { glyph: "shimmer" }),
  HD2_ReverbGlitz: rack(C.magenta, 4, { glyph: "shimmer" }),
  HD2_ReverbGanymede: rack(C.indigo, 4, { glyph: "shimmer" }),
  HD2_ReverbSearchlights: rack(C.purple, 4, { glyph: "shimmer" }),
  HD2_ReverbPlateaux: rack(C.violet, 4, { glyph: "plate" }),
  HD2_ReverbNonLinear: rack(C.orange, 3, { glyph: "gate" }),
  HD2_ReverbDucking: rack(C.amber, 3, { glyph: "gate" }),
  HD2_ReverbEcho: rack(C.forest, 3, { glyph: "echo" }),
  HD2_ReverbOcto: rack(C.mint, 3, { glyph: "shimmer" }),
  HD2_ReverbParticle: rack(C.pink, 3, { glyph: "particle" }),

  // ---- Pitch / Synth ---------------------------------------------------
  HD2_PitchPitchWham: wah(C.red),
  L6SPB_PolyWham: wah(C.crimson),
  L6SPB_PolyBassWham: wah(C.oxblood),
  HD2_PitchTwinHarmony: box(C.violet, 4, { layout: "grid", mark: "window" }),
  HD2_PitchSimplePitch: box(C.purple, 3, { mark: "window" }),
  HD2_PitchDualPitch: box(C.purple, 5, { layout: "grid", mark: "window" }),
  VIC_PitchBoctaver: box(C.indigo, 4, { layout: "grid" }),
  L6SPB_PolyPitch: box(C.purple, 4, { layout: "grid", mark: "window" }),
  L6SPB_PolyDowntune: box(C.indigo, 2, { mark: "window" }),
  L6SPB_12String: box(C.tan, 4, { layout: "grid" }),
  VIC_PitchTwelveString: box(C.tan, 4, { layout: "grid" }),
  HD2_M13TwoVoiceHarmony: box(C.violet, 5, { layout: "grid", mark: "window" }),
  HD2_DM4BassOctaver: box(C.silver, 3, { plate: C.chrome }),
  HD2_Synth3NoteGenerator: rack(C.black, 6, { layout: "grid" }),
  HD2_Synth4OSCGenerator: rack(C.black, 6, { layout: "grid" }),
  HD2_SynthSubtractive: rack(C.black, 6, { layout: "grid" }),
  HD2_FM4OctiSynth: box(C.magenta, 4, { layout: "grid" }),
  HD2_FM4SynthOMatic: box(C.magenta, 4, { layout: "grid" }),
  HD2_FM4AttackSynth: box(C.pink, 4, { layout: "grid" }),
  HD2_FM4SynthString: box(C.violet, 4, { layout: "grid" }),
  HD2_FM4Growler: box(C.purple, 4, { layout: "grid" }),

  // ---- Filter ----------------------------------------------------------
  HD2_FilterMutantFilter: wide(C.steel, 4, { plate: "#6b7079" }),
  HD2_FilterMysterFilter: wide(C.charcoal, 4),
  HD2_FilterAutoFilter: box(C.aqua, 4, { layout: "grid" }),
  HD2_FilterAshevillePattrn: box(C.indigo, 5, { layout: "grid", mark: "window" }),
  HD2_FM4VoiceBox: box(C.pink, 3),
  HD2_FM4VTron: wide(C.steel, 3),
  HD2_FM4QFilter: box(C.aqua, 3),
  HD2_FM4Seeker: box(C.teal, 4, { layout: "grid" }),
  HD2_FM4ObiWah: box(C.sky, 3),
  HD2_FM4TronUp: wide(C.steel, 3),
  HD2_FM4TronDown: wide(C.charcoal, 3),
  HD2_FM4Throbber: box(C.violet, 4, { layout: "grid" }),
  HD2_FM4SlowFilter: box(C.mint, 3),
  HD2_FM4SpinCycle: box(C.magenta, 3),
  HD2_FM4CometTrails: box(C.indigo, 4, { layout: "grid" }),

  // ---- Wah -------------------------------------------------------------
  HD2_WahUKWah846: wah(C.black),
  HD2_WahTeardrop310: wah(C.crimson, { teardrop: true }),
  HD2_WahTeardropBassQ: wah(C.navy, { teardrop: true }),
  HD2_WahFassel: wah(C.charcoal),
  HD2_WahWeeper: wah(C.graphite),
  HD2_WahChrome: wah(C.chrome),
  HD2_WahChromeCustom: wah(C.silver),
  HD2_WahThroaty: wah(C.oxblood),
  HD2_WahVettaWah: wah(C.steel),
  HD2_WahColorful: wah(C.orange),
  HD2_WahConductor: wah(C.espresso),

  // ---- Volume / Pan ----------------------------------------------------
  HD2_VolPanVol: { shape: "pedalboard", body: C.graphite },
  HD2_VolPanGain: util(1, { arrow: true }),
  HD2_VolPanPan: util(2, { arrow: true }),
  HD2_VolPanStereoWidth: util(2),
  HD2_VolPanStereoImager: util(2),

  // ---- Looper ----------------------------------------------------------
  HD2_Looper: { shape: "looper", body: C.graphite, sw: 4 },
  HD2_LooperOneSwitch: { shape: "looper", body: C.graphite, sw: 1 },
  ShufflingLooper: { shape: "looper", body: C.magenta, sw: 2 },
  VIC_LooperShuffling: { shape: "looper", body: C.magenta, sw: 2 },
};

// Send/Return/FX-Loop are generated: the number in the name is the only thing that varies.
for (let i = 1; i <= 4; i++) {
  MODELS[`HD2_SendMono${i}`] = util(1, { arrow: true, body: C.teal });
  MODELS[`HD2_ReturnMono${i}`] = util(1, { body: C.teal });
  MODELS[`HD2_FXLoopMono${i}`] = util(2, { arrow: true, body: C.teal });
}
for (const p of ["1_2", "3_4"]) {
  MODELS[`HD2_SendStereo${p}`] = util(2, { arrow: true, body: C.teal });
  MODELS[`HD2_ReturnStereo${p}`] = util(2, { body: C.teal });
  MODELS[`HD2_FXLoopStereo${p}`] = util(2, { arrow: true, body: C.teal });
}

// ---- Amps ---------------------------------------------------------------
//
// Matched on the symbolic id prefix, longest first, so `HD2_AmpBritPlexiBrt` and
// `HD2_AmpBritPlexiJump` share one entry. What identifies a head at 22px is the tolex and the
// control-panel finish — tweed with a brown panel, black with a gold panel, black with brushed
// silver — plus roughly how many knobs are on the front.

const head = (body, panel, knobs, o = {}) => ({ shape: "head", body, panel, knobs, ...o });
const combo = (body, panel, knobs, o = {}) => ({ shape: "combo", body, panel, knobs, ...o });

export const AMP_RULES = [
  // Tweed and lacquered-cabinet Americana.
  ["HD2_AmpTweedBlues", combo(C.tweed, PANEL.brown, 4, { cloth: CLOTH.wheat })],
  ["HD2_AmpUSSmallTweed", combo(C.tweed, PANEL.brown, 2, { cloth: CLOTH.wheat })],
  ["HD2_AmpFullerton", combo(C.tweed, PANEL.brown, 3, { cloth: CLOTH.wheat })],
  ["HD2_AmpGrammatico", combo(C.tweed, PANEL.brown, 3, { cloth: CLOTH.wheat })],
  ["HD2_AmpGSG100", head(C.tweed, PANEL.brown, 5, { cloth: CLOTH.wheat })],
  ["HD2_AmpDerailedIngrid", head(C.ivory, PANEL.brown, 4, { cloth: CLOTH.wheat })],
  ["HD2_AmpSoupPro", combo(C.espresso, PANEL.copper, 3, { cloth: CLOTH.cane })],
  ["HD2_AmpStoneAge185", combo(C.brown, PANEL.copper, 3, { cloth: CLOTH.cane })],
  ["HD2_AmpVoltageQueen", combo(C.tweed, PANEL.brown, 2, { cloth: CLOTH.wheat })],
  ["HD2_AmpMailOrderTwin", combo(C.espresso, PANEL.copper, 4, { cloth: CLOTH.cane })],
  ["HD2_AmpDividedDuo", combo(C.ivory, PANEL.chrome, 4, { cloth: CLOTH.wheat })],
  ["HD2_AmpInterstateZed", head(C.espresso, PANEL.cream, 4, { cloth: CLOTH.cane })],
  // Blackface / silverface.
  ["HD2_AmpUSPrincess", combo(C.black, PANEL.blackface, 4, { cloth: CLOTH.silver })],
  ["HD2_AmpUSDeluxe", combo(C.black, PANEL.blackface, 5, { cloth: CLOTH.silver })],
  ["HD2_AmpUSDouble", combo(C.black, PANEL.blackface, 6, { cloth: CLOTH.silver })],
  ["HD2_AmpUSSuper", combo(C.black, PANEL.blackface, 5, { cloth: CLOTH.silver })],
  ["HD2_AmpUSDripman", head(C.black, PANEL.silverface, 5, { cloth: CLOTH.silver })],
  ["HD2_AmpJazzRivet", combo(C.black, PANEL.silverface, 6, { cloth: CLOTH.black })],
  // British.
  ["HD2_AmpBritJ45", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpBritTrem", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpBritPlexi", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpBritP75", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpBrit2203", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpBrit2204", head(C.black, PANEL.plexi, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpWhoWatt", head(C.black, PANEL.silverface, 6, { cloth: CLOTH.basketweave })],
  ["HD2_AmpEssexA15", combo(C.espresso, PANEL.copper, 3, { cloth: CLOTH.brown })],
  ["HD2_AmpEssexA30", combo(C.espresso, PANEL.copper, 4, { cloth: CLOTH.brown })],
  ["HD2_AmpA30Fawn", combo(C.sand, PANEL.copper, 4, { cloth: CLOTH.brown })],
  ["HD2_AmpMatchstick", combo(C.ivory, PANEL.chrome, 5, { cloth: CLOTH.wheat })],
  ["HD2_AmpMandarinBass", head(C.orange, PANEL.white, 5, { cloth: CLOTH.wheat })],
  ["HD2_AmpMandarin", head(C.orange, PANEL.white, 5, { cloth: CLOTH.wheat })],
  ["HD2_AmpMoon", head(C.black, PANEL.silverface, 5, { cloth: CLOTH.black })],
  // High gain.
  ["HD2_AmpPlacater", head(C.black, PANEL.black, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCartographer", head(C.black, PANEL.chrome, 5, { cloth: CLOTH.black })],
  ["HD2_AmpGermanXtraBlue", head(C.navy, PANEL.chrome, 6, { cloth: CLOTH.blue })],
  ["HD2_AmpGermanXtraRed", head(C.crimson, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpGermanMahadeva", head(C.forest, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpGermanUbersonic", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCaliTexas", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCaliIV", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCaliRectifire", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCaliBass", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpCali400", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpArchetype", head(C.black, PANEL.black, 6, { cloth: CLOTH.black })],
  ["HD2_AmpANGL", head(C.black, PANEL.black, 6, { cloth: CLOTH.black, led: "#7fd4ff" })],
  ["HD2_AmpSoloLead", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpEVPanama", head(C.ivory, PANEL.black, 6, { cloth: CLOTH.black })],
  ["HD2_AmpPVPanama", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpPVVitriol", head(C.black, PANEL.black, 6, { cloth: CLOTH.black })],
  ["HD2_AmpRevvGenPurple", head(C.black, PANEL.black, 6, { cloth: CLOTH.black, led: "#b98cff" })],
  ["HD2_AmpRevvGenRed", head(C.black, PANEL.black, 6, { cloth: CLOTH.black, led: "#ff5c5c" })],
  ["HD2_AmpDasBenzin", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  // Bass.
  ["HD2_AmpTucknGo", combo(C.steel, PANEL.blackface, 4, { cloth: CLOTH.blue })],
  ["HD2_AmpSVBeast", head(C.black, PANEL.blackface, 6, { cloth: CLOTH.black })],
  ["HD2_AmpSVT4Pro", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpWoodyBlue", head(C.navy, PANEL.chrome, 5, { cloth: CLOTH.blue })],
  ["HD2_AmpAgua", head(C.navy, PANEL.chrome, 5, { cloth: CLOTH.blue })],
  ["HD2_AmpGCougar", head(C.black, PANEL.chrome, 6, { cloth: CLOTH.black })],
  ["HD2_AmpDelSol", head(C.charcoal, PANEL.chrome, 5, { cloth: CLOTH.black })],
  ["HD2_AmpBusyOne", head(C.black, PANEL.copper, 6, { cloth: CLOTH.black })],
  // Line 6 originals — one house look, distinguished by the red indicator.
  ["HD2_AmpLine6", head(C.charcoal, PANEL.chrome, 6, { cloth: CLOTH.black, led: "#ff5c5c" })],
];

// ---- Cabs ---------------------------------------------------------------
//
// Cab names start with the driver array ("4x12 Greenback 25"), which is the icon: four circles in a
// square. The finish comes from the family in the rest of the name.

export const CAB_FINISH = [
  [/tweed|p10r|small tweed/i, { body: C.tweed, cloth: CLOTH.wheat }],
  [/princess|us deluxe|us super|double|silver bell|blue bell|dripman/i, { body: C.black, cloth: CLOTH.silver }],
  [/greenback|brit|1960|v30|blackback|t75|uber|xxl/i, { body: C.black, cloth: CLOTH.basketweave }],
  [/mandarin/i, { body: C.orange, cloth: CLOTH.wheat }],
  [/ampeg|svt|hlf/i, { body: C.black, cloth: CLOTH.black }],
  [/match|grammatico|fullerton/i, { body: C.ivory, cloth: CLOTH.wheat }],
  [/cali|solo ?lead|cartog|guv|c90/i, { body: C.black, cloth: CLOTH.black }],
  [/woody|del sol|agua/i, { body: C.navy, cloth: CLOTH.blue }],
  [/jazz rivet|celest|lead 80|epicenter|field coil|open ca|garden|interstate|moo|whowatt|brute|em$/i, { body: C.espresso, cloth: CLOTH.brown }],
];
