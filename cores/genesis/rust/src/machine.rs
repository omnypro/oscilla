//! The machine: the chips of one VGM file, the command stream that drives
//! them and the resampler that turns their output into the caller's rate.
//!
//! Time is counted in units of 1/(44100 * 4096) s. A VGM wait is a whole
//! number of 4096-unit samples, and a chip event lands on the unit nearest
//! below its own clock cycle, which is within 6 ns of the exact time.

use std::collections::BTreeMap;
use std::sync::Arc;

use oscilla_resample::{BLEP_SHIFT, OUT_RING, Resampler};
use oscilla_vgm::{Banks, Command, SN76489, Vgm, YM2413, YM2612};

use crate::eg::Phase;
use crate::patch::{self, FmPatch, RegisterBlock};
use crate::psg::{self, Psg};
use crate::stream::{self, Stream};
use crate::ym2413::{self, Ym2413};
use crate::ym2612::Ym2612;

/// Time units in one second.
pub const UNIT: u64 = oscilla_vgm::SAMPLE_RATE as u64 * 4096;
/// Time units in one sample of the stream.
const PER_SAMPLE: u64 = 4096;

/// The volume modifier's 2^(k/32) in 16.16 fixed point, k = 0 to 31.
const GAIN: [i64; 32] = [
    65536, 66971, 68438, 69936, 71468, 73032, 74632, 76266, 77936, 79642, 81386, 83169, 84990,
    86851, 88752, 90696, 92682, 94711, 96785, 98905, 101070, 103283, 105545, 107856, 110218,
    112631, 115098, 117618, 120194, 122825, 125515, 128263,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineError {
    /// The output rate is outside 8000 to 192000 Hz.
    Rate,
    /// The file drives no chip that this core has.
    NoChip,
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MachineError::Rate => write!(f, "The output rate must be 8000 to 192000 Hz."),
            MachineError::NoChip => write!(f, "The file uses no chip that this core plays."),
        }
    }
}

impl std::error::Error for MachineError {}

/// The highest chip clock that the machine takes from a header, which is more
/// than twice the clocks of the real chips. A header can name any 30-bit clock,
/// and the work and memory of a render grow with it.
const MAX_CLOCK: u32 = 16_000_000;

/// The size of a YM2612 channel's 9-bit value in the output. A full-scale
/// channel peaks at 0.258 of full scale, as libgme renders one.
const FM_GAIN: i32 = 33;

/// The SN76489's level against the chip's own scale, in 256ths. A file whose
/// header names no FM chip plays the PSG alone at the first level. With an FM
/// chip the PSG is 0.405 of that, the relation that libgme's renders show
/// (the documents put the four channels together at one FM channel, which
/// is 0.26). Fitted to renders of one and three full-volume tones.
const PSG_GAIN_ALONE: i32 = 220;
const PSG_GAIN_WITH_FM: i32 = 89;

/// The size of a YM2413 channel's value in the output, as a ratio. A full
/// channel is about 4088 units, and 5/2 puts its peak at 0.249 of full scale,
/// which is one YM2612 channel's 0.258 within 4 percent. No reference player
/// renders a YM2413 (libgme refuses the file), and the documents give no level
/// against the YM2612, so this is a choice and not a fit.
const OPLL_GAIN: (i32, i32) = (5, 2);

/// The SN76489's level in 256ths with a YM2413 and no YM2612: the relation that
/// libgme has for any FM chip in the header.
const PSG_GAIN_WITH_OPLL: i32 = 89;

/// The default cutoff of the output high-pass. The documents give no value for
/// the console's output coupling.
const DEFAULT_HIGHPASS_HZ: f64 = 20.0;

fn psg_scaled((l, r): (i32, i32), gain: i32) -> (i32, i32) {
    ((l * gain) >> 8, (r * gain) >> 8)
}

/// The voices of the chips that are present, in the order of the channel mask.
fn layout(
    psg: &[Option<PsgSlot>; 2],
    fm: &[Option<FmSlot>; 2],
    opll: &[Option<OpllSlot>; 2],
) -> Vec<(VoiceRef, u32)> {
    let mut out = Vec::new();
    let mut add = |present: bool, bit: u32, count: usize, make: &dyn Fn(usize) -> VoiceRef| {
        if present {
            for c in 0..count {
                out.push((make(c), bit + c as u32));
            }
        }
    };
    add(fm[0].is_some(), 0, 6, &|c| VoiceRef::Fm(0, c));
    add(psg[0].is_some(), 6, 4, &|c| VoiceRef::Psg(0, c));
    add(fm[1].is_some(), 10, 6, &|c| VoiceRef::Fm(1, c));
    add(psg[1].is_some(), 16, 4, &|c| VoiceRef::Psg(1, c));
    add(opll[0].is_some(), 20, 9, &|c| VoiceRef::Opll(0, c));
    add(opll[1].is_some(), 29, 9, &|c| VoiceRef::Opll(1, c));
    out
}

fn psg_chip(second: bool) -> Chip {
    if second {
        Chip::Sn76489Second
    } else {
        Chip::Sn76489
    }
}

fn fm_chip(second: bool) -> Chip {
    if second {
        Chip::Ym2612Second
    } else {
        Chip::Ym2612
    }
}

fn opll_chip(second: bool) -> Chip {
    if second {
        Chip::Ym2413Second
    } else {
        Chip::Ym2413
    }
}

#[derive(Clone)]
struct FmSlot {
    chip: Ym2612,
    clock: u64,
    /// Samples of the chip (input clock / 144) run so far.
    samples: u64,
    level: (i32, i32),
}

#[derive(Clone)]
struct OpllSlot {
    chip: Ym2413,
    clock: u64,
    /// Samples of the chip (input clock / 72) run so far.
    samples: u64,
    level: (i32, i32),
}

#[derive(Clone)]
struct PsgSlot {
    chip: Psg,
    clock: u64,
    /// Ticks of the chip's own clock (input clock / 16) run so far.
    ticks: u64,
    /// The level that the resampler has been told.
    seen: (i32, i32),
}

/// The chips whose register writes an observer sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Sn76489,
    Sn76489Second,
    Ym2612,
    Ym2612Second,
    Ym2413,
    Ym2413Second,
}

/// Hears every register write that the machine applies to a chip, in the
/// order and at the time that the chip gets it. It cannot change the sound.
///
/// The address is 0 for the SN76489's data port and 1 for its stereo port, the
/// port number shifted left by 8 with the register for the YM2612 (the DAC's
/// 0x2A included, whether a command or a stream wrote it), and the register for
/// the YM2413. `sample` is the time in samples of 44,100 Hz.
pub trait Observer {
    fn register_write(&mut self, chip: Chip, address: u16, value: u8, sample: u64);
}

/// Holds the observer outside the machine state: a copy of the state has none.
#[derive(Default)]
struct ObserverSlot(Option<Box<dyn Observer>>);

impl Clone for ObserverSlot {
    fn clone(&self) -> ObserverSlot {
        ObserverSlot(None)
    }
}

/// What kind of sound a voice makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    Tone = 0,
    Noise = 1,
    Sample = 2,
    Fm = 3,
    Drum = 4,
}

/// One voice of one chip as a display shows it. The voices of a file are the
/// channels of the chips that it names, in the order of the channel mask: the
/// first YM2612's six, the first SN76489's four, the second YM2612's, the
/// second SN76489's, then the YM2413's nine and a second YM2413's nine.
#[derive(Clone, Debug)]
pub struct Voice {
    pub name: String,
    /// The bit of the channel mask that mutes this voice.
    pub mask_bit: u32,
    pub kind: VoiceKind,
    /// The voice is audible now: its level is above -46 dB.
    pub active: bool,
    pub key_on: bool,
    /// The pitch of the note in Hz, 0 for noise and for the DAC.
    pub frequency_hz: f32,
    /// 0 to 1: the envelope and the level register together, before the pan.
    pub volume: f32,
    /// 0 to 1: the envelope alone, or -1 when the chip has none.
    pub envelope: f32,
    /// The volume to each side, 0 to 1 (a side that the voice is not on is 0).
    pub left: f32,
    pub right: f32,
    /// An FM patch or algorithm, an instrument number, or -1.
    pub instrument: i32,
    /// 0 attack, 1 decay, 2 sustain, 3 release, or 0 where the chip has none.
    pub phase: i32,
}

/// Where a voice lives: the kind of chip, which of the two, and the channel.
#[derive(Clone, Copy, Debug)]
enum VoiceRef {
    Fm(usize, usize),
    Psg(usize, usize),
    Opll(usize, usize),
}

/// 10^(-db / 20), for a display.
fn linear(db: f32) -> f32 {
    10f32.powf(-db / 20.0)
}

/// Where the stream stands when a machine is not playing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    /// The stream is running.
    Playing,
    /// The stream ended without a loop.
    Finished,
}

#[derive(Clone)]
pub struct Machine {
    vgm: Arc<Vgm>,
    pos: usize,
    /// Time units left of the wait that the stream is in.
    wait_left: u64,
    /// The time to which the chips have run.
    now: u64,
    finished: bool,
    loops: u32,
    waited_since_loop: bool,
    /// The first and second SN76489 and YM2612 of the file.
    psg: [Option<PsgSlot>; 2],
    fm: [Option<FmSlot>; 2],
    opll: [Option<OpllSlot>; 2],
    /// The voices of the chips that the file has, with their mask bits.
    voices: Vec<(VoiceRef, u32)>,
    banks: Arc<Banks>,
    /// The read position in the PCM data bank for the YM2612's DAC.
    pcm_at: usize,
    /// The DAC streams that the file has set up, by stream number.
    streams: BTreeMap<u8, Stream>,
    out: Resampler,
    observer: ObserverSlot,
    last_event: u64,
    mul: i64,
    shift: u32,
    psg_gain: i32,
    hp_hz: f64,
    hp_a: f64,
    hp_dc: [f64; 2],
}

impl Machine {
    pub fn new(vgm: &Vgm, output_rate: i32) -> Result<Machine, MachineError> {
        let banks = Arc::new(vgm.banks());
        Machine::from_shared(Arc::new(vgm.clone()), banks, output_rate)
    }

    fn from_shared(
        vgm: Arc<Vgm>,
        banks: Arc<Banks>,
        output_rate: i32,
    ) -> Result<Machine, MachineError> {
        if !(8000..=192_000).contains(&output_rate) {
            return Err(MachineError::Rate);
        }
        // With bit 31 set, a dual SN76489 is the T6W28 of the Neo Geo Pocket,
        // which is one chip with two halves and not two chips.
        let psg_clocks = |c: &oscilla_vgm::Chip| {
            [
                Some(c.clock.min(MAX_CLOCK)),
                (c.dual && !c.variant).then_some(c.second_clock.min(MAX_CLOCK)),
            ]
        };
        let fm_clocks = |c: &oscilla_vgm::Chip| {
            [
                Some(c.clock.min(MAX_CLOCK)),
                c.dual.then_some(c.second_clock.min(MAX_CLOCK)),
            ]
        };
        let psg_clock = vgm.chip(SN76489).map_or([None; 2], psg_clocks);
        let fm_clock = vgm.chip(YM2612).map_or([None; 2], fm_clocks);
        let psg = psg_clock.map(|clock| {
            clock.map(|clock| PsgSlot {
                chip: {
                    let mut chip = Psg::new(vgm.psg_feedback, vgm.psg_width, vgm.psg_flags);
                    chip.set_tick_hz(u64::from(clock) / 16);
                    chip
                },
                clock: u64::from(clock),
                ticks: 0,
                seen: (0, 0),
            })
        });
        let fm = fm_clock.map(|clock| {
            clock.map(|clock| FmSlot {
                chip: Ym2612::new(),
                clock: u64::from(clock),
                samples: 0,
                level: (0, 0),
            })
        });
        let opll_clock = vgm.chip(YM2413).map_or([None; 2], fm_clocks);
        let opll = opll_clock.map(|clock| {
            clock.map(|clock| OpllSlot {
                chip: Ym2413::new(),
                clock: u64::from(clock),
                samples: 0,
                level: (0, 0),
            })
        });
        if psg.iter().all(Option::is_none)
            && fm.iter().all(Option::is_none)
            && opll.iter().all(Option::is_none)
        {
            return Err(MachineError::NoChip);
        }
        let m = i32::from(vgm.volume_modifier);
        let m = if m == -63 { -64 } else { m };
        let (q, r) = (m.div_euclid(32), m.rem_euclid(32));
        let fm_present = fm.iter().any(Option::is_some);
        let opll_present = opll.iter().any(Option::is_some);
        let voices = layout(&psg, &fm, &opll);
        let mut machine = Machine {
            pos: vgm.data_at(),
            wait_left: 0,
            now: 0,
            finished: false,
            loops: 0,
            waited_since_loop: false,
            psg,
            fm,
            voices,
            opll,
            banks,
            vgm,
            pcm_at: 0,
            streams: BTreeMap::new(),
            out: Resampler::with_ratio(output_rate, UNIT, 1),
            observer: ObserverSlot::default(),
            last_event: 0,
            mul: GAIN[r as usize],
            shift: (BLEP_SHIFT as i32 + 16 - q) as u32,
            psg_gain: if fm_present {
                PSG_GAIN_WITH_FM
            } else if opll_present {
                PSG_GAIN_WITH_OPLL
            } else {
                PSG_GAIN_ALONE
            },
            hp_hz: 0.0,
            hp_a: 0.0,
            hp_dc: [0.0; 2],
        };
        machine.set_highpass_hz(DEFAULT_HIGHPASS_HZ);
        Ok(machine)
    }

    /// Sets the observer, or removes it with `None`. A copy of the machine has
    /// none, and `reset` and `restore` keep the one that is set.
    pub fn set_observer(&mut self, observer: Option<Box<dyn Observer>>) {
        self.observer.0 = observer;
    }

    fn notify(&mut self, chip: Chip, address: u16, value: u8) {
        let sample = self.now / PER_SAMPLE;
        if let Some(o) = self.observer.0.as_mut() {
            o.register_write(chip, address, value, sample);
        }
    }

    /// The number of voices that the file's chips have.
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// What voice `index` is doing, or `None` for an index past the last.
    pub fn voice(&self, index: usize) -> Option<Voice> {
        let &(voice, mask_bit) = self.voices.get(index)?;
        let second = |slot: usize| if slot == 1 { " (2nd chip)" } else { "" };
        Some(match voice {
            VoiceRef::Fm(slot, c) => {
                let f = self.fm[slot].as_ref()?;
                let v = f.chip.voice(c);
                let inc = if v.block == 0 {
                    u32::from(v.fnum) >> 1
                } else {
                    u32::from(v.fnum) << (v.block - 1)
                };
                let hz = inc as f32 * (f.clock as f32 / 144.0) / 1_048_576.0;
                let volume = linear(f32::from(v.total) * 0.093_75);
                Voice {
                    name: format!("FM {}{}", c + 1, second(slot)),
                    mask_bit,
                    kind: if v.dac {
                        VoiceKind::Sample
                    } else {
                        VoiceKind::Fm
                    },
                    active: v.dac || volume > 0.005,
                    key_on: v.key_on,
                    frequency_hz: if v.dac { 0.0 } else { hz },
                    volume,
                    envelope: linear(f32::from(v.envelope) * 0.093_75),
                    left: if v.left { volume } else { 0.0 },
                    right: if v.right { volume } else { 0.0 },
                    instrument: i32::from(v.algorithm) * 8 + i32::from(v.feedback),
                    phase: match v.phase {
                        Phase::Attack => 0,
                        Phase::Decay => 1,
                        Phase::Sustain => 2,
                        Phase::Release => 3,
                    },
                }
            }
            VoiceRef::Psg(slot, c) => {
                let p = self.psg[slot].as_ref()?;
                let v = p.chip.voice(c);
                let volume = if v.attenuation >= 15 {
                    0.0
                } else {
                    linear(f32::from(v.attenuation) * 2.0)
                };
                Voice {
                    name: if v.noise {
                        format!("PSG Noise{}", second(slot))
                    } else {
                        format!("PSG Tone {}{}", c + 1, second(slot))
                    },
                    mask_bit,
                    kind: if v.noise {
                        VoiceKind::Noise
                    } else {
                        VoiceKind::Tone
                    },
                    active: volume > 0.0,
                    key_on: v.attenuation < 15,
                    frequency_hz: if v.noise || v.period == 0 {
                        0.0
                    } else {
                        p.clock as f32 / (32.0 * f32::from(v.period))
                    },
                    volume,
                    envelope: -1.0,
                    left: if v.left { volume } else { 0.0 },
                    right: if v.right { volume } else { 0.0 },
                    instrument: if v.noise {
                        i32::from(v.white) * 4 + i32::from(v.register)
                    } else {
                        -1
                    },
                    phase: 0,
                }
            }
            VoiceRef::Opll(slot, c) => {
                let o = self.opll[slot].as_ref()?;
                let v = o.chip.voice(c);
                let envelope = linear(f32::from(v.envelope) * 0.375);
                let volume = envelope * linear(f32::from(v.volume) * 3.0);
                let hz =
                    ((u32::from(v.fnum)) << v.block) as f32 * (o.clock as f32 / 72.0) / 524_288.0;
                Voice {
                    name: format!("OPLL {}{}", c + 1, second(slot)),
                    mask_bit,
                    kind: if v.drum {
                        VoiceKind::Drum
                    } else {
                        VoiceKind::Fm
                    },
                    active: volume > 0.005,
                    key_on: v.key_on,
                    frequency_hz: hz,
                    volume,
                    envelope,
                    left: volume,
                    right: volume,
                    instrument: i32::from(v.instrument),
                    phase: i32::from(v.phase),
                }
            }
        })
    }

    /// Fills `out` with the last `out.len()` frames that voice `index` put out,
    /// at the machine's output rate, as floats within -1 to 1, oldest first.
    /// The voice's own level and panning are in it, and the channel mask and
    /// the mixer are not, so a muted voice still shows. The chips keep 4,096
    /// samples of their own rates (the YM2612's 53 kHz, the YM2413's 50 kHz,
    /// 44,100 Hz for the SN76489), and frames that reach further back are 0.
    /// Returns false, and leaves `out` alone, for an index past the last voice.
    pub fn scope(&self, index: usize, out: &mut [f32]) -> bool {
        let Some(&(voice, _)) = self.voices.get(index) else {
            return false;
        };
        let (rate, scale): (f64, f64) = match voice {
            VoiceRef::Fm(slot, _) => (
                self.fm[slot]
                    .as_ref()
                    .map_or(1.0, |f| f.clock as f64 / 144.0),
                32768.0,
            ),
            VoiceRef::Psg(..) => (psg::SCOPE_HZ as f64, 8192.0),
            VoiceRef::Opll(slot, _) => (
                self.opll[slot]
                    .as_ref()
                    .map_or(1.0, |o| o.clock as f64 / 72.0),
                32768.0,
            ),
        };
        let ratio = rate / f64::from(self.out.rate());
        let get = |back: usize| -> f64 {
            f64::from(match voice {
                VoiceRef::Fm(slot, c) => self.fm[slot]
                    .as_ref()
                    .map_or(0, |f| f.chip.scope_sample(c, back)),
                VoiceRef::Psg(slot, c) => self.psg[slot]
                    .as_ref()
                    .map_or(0, |p| p.chip.scope_sample(c, back)),
                VoiceRef::Opll(slot, c) => self.opll[slot]
                    .as_ref()
                    .map_or(0, |o| o.chip.scope_sample(c, back)),
            })
        };
        let n = out.len();
        for (k, o) in out.iter_mut().enumerate() {
            let back = (n - 1 - k) as f64 * ratio;
            let i = back as usize;
            let frac = back - i as f64;
            let (a, b) = (get(i), get(i + 1));
            *o = ((a + (b - a) * frac) / scale) as f32;
        }
        true
    }

    /// The register blocks of the chips that the file has, in the order of the
    /// channel mask: the first YM2612, the first SN76489, the second YM2612,
    /// the second SN76489, then the YM2413 and a second YM2413.
    fn blocks(&self) -> impl Iterator<Item = RegisterBlock> + '_ {
        let block = |present: bool, chip, name, size| {
            present.then_some(RegisterBlock {
                chip,
                name,
                base: 0,
                size,
            })
        };
        [
            block(
                self.fm[0].is_some(),
                Chip::Ym2612,
                "YM2612",
                patch::YM2612_BLOCK,
            ),
            block(
                self.psg[0].is_some(),
                Chip::Sn76489,
                "SN76489",
                patch::SN76489_BLOCK,
            ),
            block(
                self.fm[1].is_some(),
                Chip::Ym2612Second,
                "YM2612 (second)",
                patch::YM2612_BLOCK,
            ),
            block(
                self.psg[1].is_some(),
                Chip::Sn76489Second,
                "SN76489 (second)",
                patch::SN76489_BLOCK,
            ),
            block(
                self.opll[0].is_some(),
                Chip::Ym2413,
                "YM2413",
                patch::YM2413_BLOCK,
            ),
            block(
                self.opll[1].is_some(),
                Chip::Ym2413Second,
                "YM2413 (second)",
                patch::YM2413_BLOCK,
            ),
        ]
        .into_iter()
        .flatten()
    }

    /// The number of register blocks: one for each chip that the file has.
    pub fn register_block_count(&self) -> usize {
        self.blocks().count()
    }

    /// What block `index` is, or `None` for an index past the last.
    pub fn register_block(&self, index: usize) -> Option<RegisterBlock> {
        self.blocks().nth(index)
    }

    /// The registers that block `index` holds now, or `None` for an index past
    /// the last. Reading changes nothing.
    pub fn register_block_read(&self, index: usize) -> Option<Vec<u8>> {
        let block = self.register_block(index)?;
        Some(match block.chip {
            Chip::Ym2612 => self.fm[0].as_ref()?.chip.registers().to_vec(),
            Chip::Ym2612Second => self.fm[1].as_ref()?.chip.registers().to_vec(),
            Chip::Sn76489 => self.psg[0].as_ref()?.chip.registers().to_vec(),
            Chip::Sn76489Second => self.psg[1].as_ref()?.chip.registers().to_vec(),
            Chip::Ym2413 => self.opll[0].as_ref()?.chip.registers().to_vec(),
            Chip::Ym2413Second => self.opll[1].as_ref()?.chip.registers().to_vec(),
        })
    }

    /// The FM patch that voice `index` uses now, or `None` for an index past
    /// the last voice and for a voice that is not an FM channel: the SN76489,
    /// channel 6 of a YM2612 while its DAC is on, and the YM2413's percussion
    /// channels in rhythm mode (the bass drum has a kind of its own).
    pub fn fm_patch(&self, index: usize) -> Option<FmPatch> {
        let &(voice, _) = self.voices.get(index)?;
        match voice {
            VoiceRef::Fm(slot, c) => {
                let f = self.fm[slot].as_ref()?;
                if f.chip.voice(c).dac {
                    return None;
                }
                Some(f.chip.fm_patch(c, f.clock as f64 / 144.0))
            }
            VoiceRef::Psg(..) => None,
            VoiceRef::Opll(slot, c) => {
                let o = self.opll[slot].as_ref()?;
                o.chip.fm_patch(c, o.clock as f64 / 72.0)
            }
        }
    }

    pub fn output_rate(&self) -> i32 {
        self.out.rate()
    }

    /// Passes through the loop point so far.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    pub fn end_state(&self) -> EndState {
        if self.finished {
            EndState::Finished
        } else {
            EndState::Playing
        }
    }

    /// Stream time so far in samples of 44,100 Hz.
    pub fn samples(&self) -> u64 {
        self.now / PER_SAMPLE
    }

    pub fn file(&self) -> &Vgm {
        &self.vgm
    }

    /// Starts the song again with the same file, rate, channel mask and high-pass.
    pub fn reset(&mut self) {
        let mask = self.channel_mask();
        let banks = Arc::clone(&self.banks);
        if let Ok(mut m) = Machine::from_shared(Arc::clone(&self.vgm), banks, self.out.rate()) {
            m.set_channel_mask(mask);
            m.set_highpass_hz(self.hp_hz);
            m.observer = std::mem::take(&mut self.observer);
            *self = m;
        }
    }

    /// Returns to the state of `saved`, a copy taken from this machine or one
    /// of the same file and output rate.
    pub fn restore(&mut self, saved: &Machine) -> bool {
        let same_file = Arc::ptr_eq(&self.vgm, &saved.vgm) || *self.vgm == *saved.vgm;
        if !same_file || self.out.rate() != saved.out.rate() {
            return false;
        }
        let observer = std::mem::take(&mut self.observer);
        *self = saved.clone();
        self.observer = observer;
        true
    }

    /// Chooses the channels that reach the mix: bits 0 to 5 are the YM2612's
    /// channels 1 to 6 and bits 6 to 9 the SN76489's channels 0 to 3. A file
    /// with a second YM2612 has its channels at bits 10 to 15 and a file with
    /// a second SN76489 has its channels at bits 16 to 19. A file with a
    /// YM2413 has its nine channels at bits 20 to 28 and a second one at bits
    /// 29 to 37.
    pub fn set_channel_mask(&mut self, mask: u64) {
        self.set_fm_mask((mask & 0x3F) as u8);
        self.set_psg_mask(((mask >> 6) & 0x0F) as u8);
        if let Some(f) = self.fm[1].as_mut() {
            f.chip.set_mask(((mask >> 10) & 0x3F) as u8);
        }
        if let Some(p) = self.psg[1].as_mut() {
            p.chip.set_mask(((mask >> 16) & 0x0F) as u8);
        }
        if let Some(o) = self.opll[0].as_mut() {
            o.chip.set_mask(((mask >> 20) & 0x1FF) as u16);
        }
        if let Some(o) = self.opll[1].as_mut() {
            o.chip.set_mask(((mask >> 29) & 0x1FF) as u16);
        }
        self.sync();
    }

    pub fn channel_mask(&self) -> u64 {
        let fm = |i: usize, absent: u64| {
            self.fm[i]
                .as_ref()
                .map_or(absent, |f| u64::from(f.chip.mask()))
        };
        let psg = |i: usize, absent: u64| {
            self.psg[i]
                .as_ref()
                .map_or(absent, |p| u64::from(p.chip.mask()))
        };
        let opll = |i: usize| {
            self.opll[i]
                .as_ref()
                .map_or(0, |o| u64::from(o.chip.mask()))
        };
        fm(0, 0x3F)
            | psg(0, 0x0F) << 6
            | fm(1, 0) << 10
            | psg(1, 0) << 16
            | opll(0) << 20
            | opll(1) << 29
    }

    /// What each channel of the first chips puts out now, before the mask,
    /// the panning and the mixer: the six FM channels (signed, within about
    /// 320, with the DAC in the sixth while it is on), then the SN76489's
    /// three tones and its noise (signed, within 8192). A chip that the file
    /// lacks reads as zeros.
    pub fn channel_levels(&self) -> [i32; 10] {
        let mut out = [0; 10];
        if let Some(f) = self.fm[0].as_ref() {
            out[..6].copy_from_slice(&f.chip.levels());
        }
        if let Some(p) = self.psg[0].as_ref() {
            out[6..].copy_from_slice(&p.chip.channels());
        }
        out
    }

    /// Chooses the channels of the first SN76489 that reach the mix.
    pub fn set_psg_mask(&mut self, mask: u8) {
        if let Some(p) = self.psg[0].as_mut() {
            p.chip.set_mask(mask);
        }
        self.sync();
    }

    /// Chooses the channels of the first YM2612 that reach the mix.
    pub fn set_fm_mask(&mut self, mask: u8) {
        if let Some(f) = self.fm[0].as_mut() {
            f.chip.set_mask(mask);
        }
    }

    /// The output level of the chips together as the resampler knows it.
    fn level(&self) -> (i32, i32) {
        let mut sum = (0, 0);
        for (l, r) in self
            .psg
            .iter()
            .flatten()
            .map(|p| p.seen)
            .chain(self.fm.iter().flatten().map(|f| f.level))
            .chain(self.opll.iter().flatten().map(|o| o.level))
        {
            sum = (sum.0 + l, sum.1 + r);
        }
        sum
    }

    /// Sets the resampler to the chips' level at the current time.
    fn sync(&mut self) {
        let gain = self.psg_gain;
        for p in self.psg.iter_mut().flatten() {
            p.seen = psg_scaled(p.chip.level(), gain);
        }
        let (l, r) = self.level();
        let t = self.now.max(self.last_event);
        self.out.step(t, l, r);
        self.last_event = t;
    }

    fn advance(&mut self, to: u64) {
        if self.finished {
            self.now = to;
            return;
        }
        let psg_gain = self.psg_gain;
        // The changes of level of every chip, as (time, source, left, right).
        // The sources count the SN76489s and then the YM2612s, which is the
        // order that equal times take.
        let mut events: Vec<(u64, usize, i32, i32)> = Vec::new();
        for (i, slot) in self.psg.iter_mut().enumerate() {
            let Some(p) = slot.as_mut() else { continue };
            let per_tick = 16 * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(p.clock) / per_tick) as u64;
            let (base, clock) = (p.ticks, u128::from(p.clock));
            p.chip.run(want - base, |t, l, r| {
                let at = (u128::from(base + t) * per_tick / clock) as u64;
                let (l, r) = psg_scaled((l, r), psg_gain);
                events.push((at, i, l, r));
            });
            p.ticks = want;
        }
        for (i, slot) in self.fm.iter_mut().enumerate() {
            let Some(f) = slot.as_mut() else { continue };
            let per_sample = 144 * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(f.clock) / per_sample) as u64;
            while f.samples < want {
                f.samples += 1;
                let (l, r) = f.chip.sample();
                let at = (u128::from(f.samples) * per_sample / u128::from(f.clock)) as u64;
                events.push((at, 2 + i, l * FM_GAIN, r * FM_GAIN));
            }
        }
        for (i, slot) in self.opll.iter_mut().enumerate() {
            let Some(o) = slot.as_mut() else { continue };
            let per_sample = u128::from(ym2413::CYCLES_PER_SAMPLE) * u128::from(UNIT);
            let want = (u128::from(to) * u128::from(o.clock) / per_sample) as u64;
            while o.samples < want {
                o.samples += 1;
                let v = o.chip.sample() * OPLL_GAIN.0 / OPLL_GAIN.1;
                let at = (u128::from(o.samples) * per_sample / u128::from(o.clock)) as u64;
                events.push((at, 4 + i, v, v));
            }
        }
        events.sort_by_key(|e| (e.0, e.1));
        let mut levels = [(0, 0); 6];
        for (i, p) in self.psg.iter().enumerate() {
            levels[i] = p.as_ref().map_or((0, 0), |p| p.seen);
        }
        for (i, f) in self.fm.iter().enumerate() {
            levels[2 + i] = f.as_ref().map_or((0, 0), |f| f.level);
        }
        for (i, o) in self.opll.iter().enumerate() {
            levels[4 + i] = o.as_ref().map_or((0, 0), |o| o.level);
        }
        for (at, source, l, r) in events {
            levels[source] = (l, r);
            let (l, r) = levels.iter().fold((0, 0), |a, v| (a.0 + v.0, a.1 + v.1));
            let at = at.max(self.last_event);
            self.out.step(at, l, r);
            self.last_event = at;
        }
        for (i, p) in self.psg.iter_mut().enumerate() {
            if let Some(p) = p.as_mut() {
                p.seen = levels[i];
            }
        }
        for (i, f) in self.fm.iter_mut().enumerate() {
            if let Some(f) = f.as_mut() {
                f.level = levels[2 + i];
            }
        }
        for (i, o) in self.opll.iter_mut().enumerate() {
            if let Some(o) = o.as_mut() {
                o.level = levels[4 + i];
            }
        }
        self.now = to;
    }

    fn finish(&mut self) {
        self.finished = true;
        let t = self.now.max(self.last_event);
        self.out.step(t, 0, 0);
        self.last_event = t;
    }

    /// Reads commands until one starts a wait or the stream ends.
    fn read_commands(&mut self) {
        let vgm = Arc::clone(&self.vgm);
        loop {
            let mut cursor = vgm.commands_from(self.pos);
            let command = cursor.read();
            match command {
                Command::End => {
                    match vgm.loop_at() {
                        Some(at) if self.waited_since_loop || self.loops == 0 => {
                            self.pos = at;
                            self.loops += 1;
                            self.waited_since_loop = false;
                        }
                        _ => self.finish(),
                    }
                    if self.finished {
                        return;
                    }
                    continue;
                }
                Command::Wait(n) => {
                    self.pos = cursor.position();
                    if n > 0 {
                        self.wait_left = u64::from(n) * PER_SAMPLE;
                        self.waited_since_loop = true;
                        return;
                    }
                }
                Command::Psg { second, value } => {
                    self.pos = cursor.position();
                    if let Some(p) = self.psg[usize::from(second)].as_mut() {
                        p.chip.write(value);
                        self.notify(psg_chip(second), 0, value);
                    }
                    self.sync();
                }
                Command::PsgStereo { second, value } => {
                    self.pos = cursor.position();
                    if let Some(p) = self.psg[usize::from(second)].as_mut() {
                        p.chip.set_stereo(value);
                        self.notify(psg_chip(second), 1, value);
                    }
                    self.sync();
                }
                Command::Ym2612 {
                    second,
                    port,
                    register,
                    value,
                } => {
                    self.pos = cursor.position();
                    if let Some(f) = self.fm[usize::from(second)].as_mut() {
                        f.chip.write(port, register, value);
                        let address = u16::from(port) << 8 | u16::from(register);
                        self.notify(fm_chip(second), address, value);
                    }
                }
                Command::Ym2612Dac { wait } => {
                    self.pos = cursor.position();
                    let byte = self.banks.bank(0).get(self.pcm_at).copied().unwrap_or(0x80);
                    self.pcm_at += 1;
                    if let Some(f) = self.fm[0].as_mut() {
                        f.chip.write(0, 0x2A, byte);
                        self.notify(Chip::Ym2612, 0x2A, byte);
                    }
                    if wait > 0 {
                        self.wait_left = u64::from(wait) * PER_SAMPLE;
                        self.waited_since_loop = true;
                        return;
                    }
                }
                Command::Ym2413 {
                    second,
                    register,
                    value,
                } => {
                    self.pos = cursor.position();
                    if let Some(o) = self.opll[usize::from(second)].as_mut() {
                        o.chip.write(register, value);
                        self.notify(opll_chip(second), u16::from(register), value);
                    }
                }
                Command::Seek(at) => {
                    self.pos = cursor.position();
                    self.pcm_at = at as usize;
                }
                Command::StreamSetup {
                    stream,
                    chip,
                    second,
                    port,
                    register,
                } => {
                    self.pos = cursor.position();
                    if stream != 0xFF {
                        self.streams
                            .insert(stream, Stream::new(chip, second, port, register));
                    }
                }
                Command::StreamData {
                    stream,
                    bank,
                    step,
                    base,
                } => {
                    self.pos = cursor.position();
                    if let Some(s) = self.streams.get_mut(&stream) {
                        s.set_data(bank, step, base);
                    }
                }
                Command::StreamFrequency { stream, hz } => {
                    self.pos = cursor.position();
                    let now = self.now;
                    if let Some(s) = self.streams.get_mut(&stream) {
                        s.set_frequency(now, hz);
                    }
                }
                Command::StreamStart {
                    stream,
                    at,
                    mode,
                    length,
                } => {
                    self.pos = cursor.position();
                    let now = self.now;
                    if let Some(s) = self.streams.get_mut(&stream) {
                        s.start(now, at, mode, length);
                    }
                }
                Command::StreamFast {
                    stream,
                    block,
                    flags,
                } => {
                    self.pos = cursor.position();
                    let now = self.now;
                    if let Some(s) = self.streams.get_mut(&stream) {
                        let block = self.banks.blocks(s.bank()).get(usize::from(block)).copied();
                        s.start_block(now, block, flags);
                    }
                }
                Command::StreamStop { stream } => {
                    self.pos = cursor.position();
                    if stream == 0xFF {
                        self.streams.values_mut().for_each(Stream::stop);
                    } else if let Some(s) = self.streams.get_mut(&stream) {
                        s.stop();
                    }
                }
                _ => self.pos = cursor.position(),
            }
        }
    }

    /// The time of the next stream write.
    fn next_stream_write(&self) -> Option<u64> {
        self.streams.values().filter_map(Stream::due).min()
    }

    /// Makes the stream writes that are due at the current time.
    fn write_streams(&mut self) {
        let now = self.now;
        let ids: Vec<u8> = self.streams.keys().copied().collect();
        for id in ids {
            while let Some(s) = self.streams.get_mut(&id) {
                if s.due().is_none_or(|t| t > now) {
                    break;
                }
                let byte = s.next(self.banks.bank(s.bank()));
                let (chip, second, port, register) = (s.chip, s.second, s.port, s.register);
                if let Some(byte) = byte
                    && chip == stream::YM2612
                    && let Some(f) = self.fm[usize::from(second)].as_mut()
                {
                    f.chip.write(port, register, byte);
                    let address = u16::from(port) << 8 | u16::from(register);
                    self.notify(fm_chip(second), address, byte);
                }
            }
        }
    }

    fn run_until(&mut self, target: u64) {
        while self.now < target {
            if self.finished {
                self.advance(target);
                return;
            }
            self.write_streams();
            if self.wait_left == 0 {
                self.read_commands();
                continue;
            }
            let mut step = self.wait_left.min(target - self.now);
            if let Some(t) = self.next_stream_write() {
                step = step.min(t - self.now);
            }
            self.advance(self.now + step);
            self.wait_left -= step;
        }
    }

    /// Runs the machine until the resampler holds `frames` more frames.
    fn produce(&mut self, frames: usize) {
        let rate = self.out.rate() as u64;
        while self.out.available() < frames {
            let missing = (frames - self.out.available()) as u64;
            let target = self.now + (missing * UNIT).div_ceil(rate).max(1);
            self.run_until(target);
            let t = self.now;
            self.out.finalize(t);
        }
    }

    /// Fills `out` with interleaved left and right samples.
    pub fn render_s16(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut at = 0;
        let mut tmp = [0f32; OUT_RING / 2];
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            let got = self.take_f32(&mut tmp[..2 * chunk], chunk);
            for (o, v) in out[2 * at..2 * (at + got)].iter_mut().zip(&tmp) {
                *o = (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            }
            at += got;
        }
    }

    /// Fills `out` with interleaved left and right floats.
    pub fn render_f32(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut at = 0;
        while at < frames {
            let chunk = (frames - at).min(OUT_RING / 4);
            at += self.take_f32(&mut out[2 * at..2 * (at + chunk)], chunk);
        }
    }

    fn take_f32(&mut self, out: &mut [f32], chunk: usize) -> usize {
        // The volume modifier's whole octaves, as an exact power of two.
        let octave = BLEP_SHIFT as i32 + 16 - self.shift as i32;
        let power = if octave >= 0 {
            (1u64 << octave.min(62)) as f64
        } else {
            1.0 / (1u64 << (-octave).min(62)) as f64
        };
        let gain = self.mul as f64 / 65536.0 * power;
        let scale = gain / f64::from(1u32 << 15) / f64::from(1u32 << BLEP_SHIFT);
        self.produce(chunk);
        let got = self.out.take_f32(out, chunk, scale);
        if self.hp_a > 0.0 {
            for frame in out[..2 * got].chunks_exact_mut(2) {
                for (v, dc) in frame.iter_mut().zip(self.hp_dc.iter_mut()) {
                    let x = f64::from(*v);
                    *dc += (x - *dc) * self.hp_a;
                    *v = (x - *dc) as f32;
                }
            }
        }
        got
    }

    /// Sets the cutoff of the high-pass that stands for the console's output
    /// coupling and removes the DC offset that the DAC can leave. 0 turns it
    /// off. Returns false, and changes nothing, for a value outside 0 to 1000 Hz.
    pub fn set_highpass_hz(&mut self, hz: f64) -> bool {
        if !(0.0..=1000.0).contains(&hz) {
            return false;
        }
        self.hp_hz = hz;
        self.hp_a = 2.0 * std::f64::consts::PI * hz / f64::from(self.out.rate());
        true
    }

    pub fn highpass_hz(&self) -> f64 {
        self.hp_hz
    }
}
