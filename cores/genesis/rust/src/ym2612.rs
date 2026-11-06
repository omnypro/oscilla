//! The YM2612: six FM channels of four operators each, a low frequency
//! oscillator, two timers and a DAC on channel 6. One call of `sample` is one
//! output sample of the chip, 144 clocks of its input.

use crate::eg::Eg;

/// Samples between two steps of the LFO for each rate. A step is 1/128 of a
/// period.
const LFO_PERIOD: [u32; 8] = [109, 78, 72, 68, 63, 45, 9, 6];

/// The peak shift of the F-number at each phase modulation depth, in units of
/// 1/1024 of an F-number of 1024.
const PM_PEAK: [i32; 8] = [0, 2, 4, 6, 8, 12, 24, 48];

/// The DAC's size against an FM channel, in quarters. libgme's render of a
/// full-scale DAC square is 1.8 dB above its render of a full-scale FM sine by
/// RMS, where the documents give both the same 9-bit range.
const DAC_GAIN: i32 = 5;

/// How far a modulator's output is shifted down before it enters the 10-bit
/// phase of the operator that it modulates.
pub const MOD_SHIFT: u32 = 1;

/// The operator behind each position in a group of registers (S1, S3, S2, S4).
const REGISTER_ORDER: [usize; 4] = [0, 2, 1, 3];

/// The offset that the ladder effect adds below zero in a channel's 9-bit
/// value, when the option is on.
const LADDER: i32 = 3;

/// The log-sine of the first quarter wave in 4.8 fixed point, and the
/// exponent table. The generating formulas are
/// round(-log2(sin((x + 0.5) * pi / 512)) * 256) and
/// round(2^((255 - i) / 256) * 1024).
const LOGSIN: [u16; 256] = [
    2137, 1731, 1543, 1419, 1326, 1252, 1190, 1137, 1091, 1050, 1013, 979, 949, 920, 894, 869, 846,
    825, 804, 785, 767, 749, 732, 717, 701, 687, 672, 659, 646, 633, 621, 609, 598, 587, 576, 566,
    556, 546, 536, 527, 518, 509, 501, 492, 484, 476, 468, 461, 453, 446, 439, 432, 425, 418, 411,
    405, 399, 392, 386, 380, 375, 369, 363, 358, 352, 347, 341, 336, 331, 326, 321, 316, 311, 307,
    302, 297, 293, 289, 284, 280, 276, 271, 267, 263, 259, 255, 251, 248, 244, 240, 236, 233, 229,
    226, 222, 219, 215, 212, 209, 205, 202, 199, 196, 193, 190, 187, 184, 181, 178, 175, 172, 169,
    167, 164, 161, 159, 156, 153, 151, 148, 146, 143, 141, 138, 136, 134, 131, 129, 127, 125, 122,
    120, 118, 116, 114, 112, 110, 108, 106, 104, 102, 100, 98, 96, 94, 92, 91, 89, 87, 85, 83, 82,
    80, 78, 77, 75, 74, 72, 70, 69, 67, 66, 64, 63, 62, 60, 59, 57, 56, 55, 53, 52, 51, 49, 48, 47,
    46, 45, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 23,
    22, 21, 20, 20, 19, 18, 17, 17, 16, 15, 15, 14, 13, 13, 12, 12, 11, 10, 10, 9, 9, 8, 8, 7, 7,
    7, 6, 6, 5, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];

const POW: [u16; 256] = [
    2042, 2037, 2031, 2026, 2020, 2015, 2010, 2004, 1999, 1993, 1988, 1983, 1977, 1972, 1966, 1961,
    1956, 1951, 1945, 1940, 1935, 1930, 1924, 1919, 1914, 1909, 1904, 1898, 1893, 1888, 1883, 1878,
    1873, 1868, 1863, 1858, 1853, 1848, 1843, 1838, 1833, 1828, 1823, 1818, 1813, 1808, 1803, 1798,
    1794, 1789, 1784, 1779, 1774, 1769, 1765, 1760, 1755, 1750, 1746, 1741, 1736, 1732, 1727, 1722,
    1717, 1713, 1708, 1704, 1699, 1694, 1690, 1685, 1681, 1676, 1672, 1667, 1663, 1658, 1654, 1649,
    1645, 1640, 1636, 1631, 1627, 1623, 1618, 1614, 1609, 1605, 1601, 1596, 1592, 1588, 1584, 1579,
    1575, 1571, 1566, 1562, 1558, 1554, 1550, 1545, 1541, 1537, 1533, 1529, 1525, 1520, 1516, 1512,
    1508, 1504, 1500, 1496, 1492, 1488, 1484, 1480, 1476, 1472, 1468, 1464, 1460, 1456, 1452, 1448,
    1444, 1440, 1436, 1433, 1429, 1425, 1421, 1417, 1413, 1409, 1406, 1402, 1398, 1394, 1391, 1387,
    1383, 1379, 1376, 1372, 1368, 1364, 1361, 1357, 1353, 1350, 1346, 1342, 1339, 1335, 1332, 1328,
    1324, 1321, 1317, 1314, 1310, 1307, 1303, 1300, 1296, 1292, 1289, 1286, 1282, 1279, 1275, 1272,
    1268, 1265, 1261, 1258, 1255, 1251, 1248, 1244, 1241, 1238, 1234, 1231, 1228, 1224, 1221, 1218,
    1214, 1211, 1208, 1205, 1201, 1198, 1195, 1192, 1188, 1185, 1182, 1179, 1176, 1172, 1169, 1166,
    1163, 1160, 1157, 1154, 1150, 1147, 1144, 1141, 1138, 1135, 1132, 1129, 1126, 1123, 1120, 1117,
    1114, 1111, 1108, 1105, 1102, 1099, 1096, 1093, 1090, 1087, 1084, 1081, 1078, 1075, 1072, 1069,
    1066, 1064, 1061, 1058, 1055, 1052, 1049, 1046, 1044, 1041, 1038, 1035, 1032, 1030, 1027, 1024,
];

/// Detune magnitudes by key code and the low two bits of the detune code.
const DETUNE: [[u8; 4]; 32] = [
    [0, 0, 1, 2],
    [0, 0, 1, 2],
    [0, 0, 1, 2],
    [0, 0, 1, 2],
    [0, 1, 2, 2],
    [0, 1, 2, 3],
    [0, 1, 2, 3],
    [0, 1, 2, 3],
    [0, 1, 2, 4],
    [0, 1, 3, 4],
    [0, 1, 3, 4],
    [0, 1, 3, 5],
    [0, 2, 4, 5],
    [0, 2, 4, 6],
    [0, 2, 4, 6],
    [0, 2, 5, 7],
    [0, 2, 5, 8],
    [0, 3, 6, 8],
    [0, 3, 6, 9],
    [0, 3, 7, 10],
    [0, 4, 8, 11],
    [0, 4, 8, 12],
    [0, 4, 9, 13],
    [0, 5, 10, 14],
    [0, 5, 11, 16],
    [0, 6, 12, 17],
    [0, 6, 13, 19],
    [0, 7, 14, 20],
    [0, 8, 16, 22],
    [0, 8, 16, 22],
    [0, 8, 16, 22],
    [0, 8, 16, 22],
];

#[derive(Clone, Copy, Default)]
struct Slot {
    eg: Eg,
    dt: u8,
    mul: u8,
    phase: u32,
    manual_key: bool,
}

#[derive(Clone, Copy)]
struct Channel {
    slots: [Slot; 4],
    fnum: u16,
    block: u8,
    high: u8,
    fb: u8,
    alg: u8,
    left: bool,
    right: bool,
    ams: u8,
    pms: u8,
    /// The first slot's outputs of the last two samples and the second
    /// slot's of the last one.
    s1_prev: i32,
    s1_prev2: i32,
    s2_prev: i32,
    level: i32,
}

impl Default for Channel {
    fn default() -> Self {
        Channel {
            slots: [Slot::default(); 4],
            fnum: 0,
            block: 0,
            high: 0,
            fb: 0,
            alg: 0,
            left: true,
            right: true,
            ams: 0,
            pms: 0,
            s1_prev: 0,
            s1_prev2: 0,
            s2_prev: 0,
            level: 0,
        }
    }
}

/// The status flags of the timers.
pub const STATUS_A: u8 = 1;
pub const STATUS_B: u8 = 2;

#[derive(Clone)]
pub struct Ym2612 {
    ch: [Channel; 6],
    /// The F-numbers and blocks of channel 3's special mode, by the
    /// registers $A8 to $AA and $AC to $AE.
    special: [(u16, u8); 3],
    special_high: [u8; 3],
    mode: u8,
    timer_a: u16,
    timer_b: u8,
    a_left: u32,
    b_left: u32,
    b_prescale: u32,
    load_a: bool,
    load_b: bool,
    enable_a: bool,
    enable_b: bool,
    status: u8,
    csm_key: bool,
    lfo_on: bool,
    lfo_rate: u8,
    lfo_wait: u32,
    lfo_step: u32,
    eg_counter: u32,
    eg_divider: u32,
    dac_on: bool,
    /// The value of $2A. Files turn the DAC on before the first sample, so the
    /// start value must play as silence (the documents give no power-on value).
    dac: u8,
    dac_low: bool,
    mask: u8,
    ladder: bool,
}

impl Default for Ym2612 {
    fn default() -> Self {
        Ym2612::new()
    }
}

/// An unsigned count of the half-wave: the first quarter mirrored, then
/// negated.
fn operator(phase: u32, modulation: i32, attenuation: u16) -> i32 {
    let p = (phase as i32 + modulation) as u32 & 0x3FF;
    let mut x = (p & 0xFF) as usize;
    if p & 0x100 != 0 {
        x = 255 - x;
    }
    let a = u32::from(LOGSIN[x]) + (u32::from(attenuation) << 2);
    let shift = a >> 8;
    let magnitude = if shift >= 13 {
        0
    } else {
        (i32::from(POW[(a & 0xFF) as usize]) << 2) >> shift
    };
    if p & 0x200 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// The 10-bit phase input from the sum of one or two operator outputs.
fn modulate(sum: i32) -> i32 {
    (sum >> MOD_SHIFT) & 0x3FF
}

fn key_code(fnum: u16, block: u8) -> u8 {
    let f = u32::from(fnum);
    let (b10, b9, b8, b7) = ((f >> 10) & 1, (f >> 9) & 1, (f >> 8) & 1, (f >> 7) & 1);
    let n4 = b10;
    let n3 = (b10 & (b9 | b8 | b7)) | ((b10 ^ 1) & b9 & b8 & b7);
    block << 2 | (n4 << 1 | n3) as u8
}

impl Ym2612 {
    pub fn new() -> Ym2612 {
        Ym2612 {
            ch: [Channel::default(); 6],
            special: [(0, 0); 3],
            special_high: [0; 3],
            mode: 0,
            timer_a: 0,
            timer_b: 0,
            a_left: 0,
            b_left: 0,
            b_prescale: 0,
            load_a: false,
            load_b: false,
            enable_a: false,
            enable_b: false,
            status: 0,
            csm_key: false,
            lfo_on: false,
            lfo_rate: 0,
            lfo_wait: 0,
            lfo_step: 0,
            eg_counter: 0,
            eg_divider: 0,
            dac_on: false,
            dac: 0x80,
            dac_low: false,
            mask: 0x3F,
            ladder: false,
        }
    }

    /// Chooses which of the six channels reach the mix. Bit n is channel n + 1.
    pub fn set_mask(&mut self, mask: u8) {
        self.mask = mask & 0x3F;
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// Adds the offset below zero that the DAC's resistor ladder produces.
    pub fn set_ladder(&mut self, on: bool) {
        self.ladder = on;
    }

    /// The timer flags that a status read returns, without the busy bit.
    pub fn status(&self) -> u8 {
        self.status
    }

    /// Each channel's 9-bit value at the last sample, before the mask and the
    /// panning.
    pub fn levels(&self) -> [i32; 6] {
        let mut out = [0; 6];
        for (o, c) in out.iter_mut().zip(&self.ch) {
            *o = c.level;
        }
        out
    }

    /// A write to a data port. `part` is 0 for the first bank of registers
    /// and 1 for the second.
    pub fn write(&mut self, part: u8, register: u8, value: u8) {
        match register {
            0x22 if part == 0 => {
                self.lfo_on = value & 8 != 0;
                self.lfo_rate = value & 7;
                if !self.lfo_on {
                    self.lfo_wait = 0;
                    self.lfo_step = 0;
                }
            }
            0x24 if part == 0 => self.timer_a = self.timer_a & 3 | u16::from(value) << 2,
            0x25 if part == 0 => self.timer_a = self.timer_a & !3 | u16::from(value & 3),
            0x26 if part == 0 => self.timer_b = value,
            0x27 if part == 0 => self.write_mode(value),
            0x28 if part == 0 => {
                let code = usize::from(value & 7);
                if code != 3 && code != 7 {
                    let c = if code >= 4 { code - 1 } else { code };
                    for op in 0..4 {
                        self.ch[c].slots[op].manual_key = value & (0x10 << op) != 0;
                    }
                }
            }
            0x2A if part == 0 => self.dac = value,
            0x2B if part == 0 => self.dac_on = value & 0x80 != 0,
            0x2C if part == 0 => self.dac_low = value & 8 != 0,
            0x30..=0xB6 => self.write_channel(part, register, value),
            _ => {}
        }
    }

    fn write_mode(&mut self, value: u8) {
        self.mode = value >> 6;
        self.enable_a = value & 4 != 0;
        self.enable_b = value & 8 != 0;
        let (load_a, load_b) = (value & 1 != 0, value & 2 != 0);
        if load_a && !self.load_a {
            self.a_left = 1024 - u32::from(self.timer_a);
        }
        if load_b && !self.load_b {
            self.b_left = 256 - u32::from(self.timer_b);
            self.b_prescale = 0;
        }
        self.load_a = load_a;
        self.load_b = load_b;
        if value & 0x10 != 0 {
            self.status &= !STATUS_A;
        }
        if value & 0x20 != 0 {
            self.status &= !STATUS_B;
        }
    }

    fn write_channel(&mut self, part: u8, register: u8, value: u8) {
        let low = usize::from(register & 3);
        if low == 3 {
            return;
        }
        let c = low + 3 * usize::from(part);
        match register {
            0x30..=0x9F => {
                let op = REGISTER_ORDER[usize::from((register >> 2) & 3)];
                let slot = &mut self.ch[c].slots[op];
                match register & 0xF0 {
                    0x30 => {
                        slot.dt = (value >> 4) & 7;
                        slot.mul = value & 15;
                    }
                    0x40 => slot.eg.params.tl = value & 0x7F,
                    0x50 => {
                        slot.eg.params.ks = value >> 6;
                        slot.eg.params.ar = value & 31;
                    }
                    0x60 => {
                        slot.eg.params.am = value & 0x80 != 0;
                        slot.eg.params.dr = value & 31;
                    }
                    0x70 => slot.eg.params.sr = value & 31,
                    0x80 => {
                        slot.eg.params.sl = value >> 4;
                        slot.eg.params.rr = value & 15;
                    }
                    _ => slot.eg.params.ssg = value & 15,
                }
            }
            0xA0..=0xA2 => {
                let ch = &mut self.ch[c];
                ch.fnum = u16::from(ch.high & 7) << 8 | u16::from(value);
                ch.block = (ch.high >> 3) & 7;
            }
            0xA4..=0xA6 => self.ch[c].high = value,
            0xA8..=0xAA if part == 0 => {
                self.special[low] = (
                    u16::from(self.special_high[low] & 7) << 8 | u16::from(value),
                    (self.special_high[low] >> 3) & 7,
                );
            }
            0xAC..=0xAE if part == 0 => self.special_high[low] = value,
            0xB0..=0xB2 => {
                self.ch[c].fb = (value >> 3) & 7;
                self.ch[c].alg = value & 7;
            }
            0xB4..=0xB6 => {
                let ch = &mut self.ch[c];
                ch.left = value & 0x80 != 0;
                ch.right = value & 0x40 != 0;
                ch.ams = (value >> 4) & 3;
                ch.pms = value & 7;
            }
            _ => {}
        }
    }

    /// The F-number and block that operator `op` of channel `c` runs at.
    fn frequency(&self, c: usize, op: usize) -> (u16, u8) {
        if c == 2 && self.mode != 0 {
            let index = match op {
                0 => Some(1),
                1 => Some(2),
                2 => Some(0),
                _ => None,
            };
            if let Some(i) = index {
                return self.special[i];
            }
        }
        (self.ch[c].fnum, self.ch[c].block)
    }

    fn step_timers(&mut self) {
        if self.load_a {
            self.a_left = self.a_left.saturating_sub(1);
            if self.a_left == 0 {
                self.a_left = 1024 - u32::from(self.timer_a);
                if self.enable_a {
                    self.status |= STATUS_A;
                }
                if self.mode == 2 {
                    self.csm_key = true;
                }
            }
        }
        if self.load_b {
            self.b_prescale += 1;
            if self.b_prescale == 16 {
                self.b_prescale = 0;
                self.b_left = self.b_left.saturating_sub(1);
                if self.b_left == 0 {
                    self.b_left = 256 - u32::from(self.timer_b);
                    if self.enable_b {
                        self.status |= STATUS_B;
                    }
                }
            }
        }
    }

    fn step_lfo(&mut self) {
        if !self.lfo_on {
            return;
        }
        self.lfo_wait += 1;
        if self.lfo_wait >= LFO_PERIOD[usize::from(self.lfo_rate)] {
            self.lfo_wait = 0;
            self.lfo_step = (self.lfo_step + 1) & 127;
        }
    }

    /// The LFO's amplitude value, 0 to 126, and its signed phase value, -8 to
    /// 8. Both are 0 while the LFO is off.
    fn lfo_values(&self) -> (u16, i32) {
        if !self.lfo_on {
            return (0, 0);
        }
        let s = self.lfo_step;
        let am = if s < 64 { 63 - s } else { s - 64 } as u16 * 2;
        let q = (s >> 2) as i32;
        let pm = if q <= 8 {
            q
        } else if q <= 24 {
            16 - q
        } else {
            q - 32
        };
        (am, pm)
    }

    /// Runs one sample of the chip and returns the left and right sums of the
    /// channels' 9-bit values.
    pub fn sample(&mut self) -> (i32, i32) {
        self.step_timers();
        self.step_lfo();
        let (am, pm) = self.lfo_values();

        // Key events, then the SSG-EG work, then the envelope update every
        // third sample.
        let csm = self.csm_key && self.mode == 2;
        self.eg_divider += 1;
        let eg_tick = self.eg_divider == 3;
        if eg_tick {
            self.eg_divider = 0;
            self.eg_counter = self.eg_counter.wrapping_add(1);
        }
        for c in 0..6 {
            for op in 0..4 {
                let (fnum, block) = self.frequency(c, op);
                let kc = key_code(fnum, block);
                let key = self.ch[c].slots[op].manual_key || (c == 2 && csm);
                let slot = &mut self.ch[c].slots[op];
                slot.eg.set_key(key, kc);
                slot.eg.ssg_step(kc);
                if eg_tick {
                    slot.eg.update(self.eg_counter, kc);
                }
                if slot.eg.take_phase_reset() {
                    slot.phase = 0;
                }
            }
        }
        self.csm_key = false;

        let (mut left, mut right) = (0, 0);
        for c in 0..6 {
            let value = self.channel(c, am, pm);
            self.ch[c].level = value;
            if self.mask & (1 << c) == 0 {
                continue;
            }
            if self.ch[c].left {
                left += value;
            }
            if self.ch[c].right {
                right += value;
            }
        }
        (left, right)
    }

    fn channel(&mut self, c: usize, am: u16, pm: i32) -> i32 {
        let (ams, pms) = (self.ch[c].ams, self.ch[c].pms);
        let am = match ams {
            0 => 0,
            1 => am >> 3,
            2 => am >> 1,
            _ => am,
        };
        let mut phase = [0u32; 4];
        let mut att = [0u16; 4];
        for op in 0..4 {
            let (fnum, block) = self.frequency(c, op);
            let mut f = i32::from(fnum);
            if pms != 0 && pm != 0 {
                let magnitude = ((f >> 4) * PM_PEAK[usize::from(pms)] * pm.abs()) >> 9;
                f = (f + if pm < 0 { -magnitude } else { magnitude }) & 0x7FF;
            }
            let mut inc = if block == 0 { f >> 1 } else { f << (block - 1) } as u32;
            let kc = key_code(fnum, block);
            let slot = &mut self.ch[c].slots[op];
            let d = u32::from(DETUNE[usize::from(kc)][usize::from(slot.dt & 3)]);
            inc = if slot.dt & 4 != 0 {
                inc.wrapping_sub(d)
            } else {
                inc + d
            } & 0x1FFFF;
            inc = if slot.mul == 0 {
                inc >> 1
            } else {
                (inc * u32::from(slot.mul)) & 0xFFFFF
            };
            slot.phase = (slot.phase + inc) & 0xFFFFF;
            phase[op] = slot.phase >> 10;
            att[op] = slot.eg.output(am);
        }

        let ch = &self.ch[c];
        let (p1, p2, s2p) = (ch.s1_prev, ch.s1_prev2, ch.s2_prev);
        let feedback = if ch.fb == 0 {
            0
        } else {
            ((p1 + p2) >> (10 - u32::from(ch.fb))) & 0x3FF
        };
        let x1 = operator(phase[0], feedback, att[0]);
        let m3 = match ch.alg {
            0 | 2 => modulate(s2p),
            1 => modulate(p1 + s2p),
            5 => modulate(p1),
            _ => 0,
        };
        let x3 = operator(phase[2], m3, att[2]);
        let m2 = match ch.alg {
            0 | 3 | 4 | 5 | 6 => modulate(x1),
            _ => 0,
        };
        let x2 = operator(phase[1], m2, att[1]);
        let m4 = match ch.alg {
            0 | 1 | 4 => modulate(x3),
            2 => modulate(x1 + x3),
            3 => modulate(s2p + x3),
            5 => modulate(x1),
            _ => 0,
        };
        let x4 = operator(phase[3], m4, att[3]);

        let carriers = match ch.alg {
            0..=3 => x4 >> 5,
            4 => (x2 >> 5) + (x4 >> 5),
            5 | 6 => (x2 >> 5) + (x3 >> 5) + (x4 >> 5),
            _ => (x1 >> 5) + (x2 >> 5) + (x3 >> 5) + (x4 >> 5),
        };
        let ch = &mut self.ch[c];
        ch.s1_prev2 = p1;
        ch.s1_prev = x1;
        ch.s2_prev = x2;

        let mut value = carriers.clamp(-256, 255);
        if c == 5 && self.dac_on {
            value = (((i32::from(self.dac) << 1) | i32::from(self.dac_low)) - 256) * DAC_GAIN / 4;
        }
        if self.ladder && value < 0 {
            value -= LADDER;
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_operator_gives_the_documented_peaks_for_a_few_levels() {
        for (tl, peak) in [(0u16, 8168), (0x3F, 34), (0x40, 31), (0x7F, 0)] {
            assert_eq!(operator(0x100, 0, tl << 3), peak, "TL {tl:#x}");
            assert_eq!(operator(0x300, 0, tl << 3), -peak, "TL {tl:#x}");
        }
    }

    #[test]
    fn the_sine_is_symmetric_and_zero_at_the_crossings() {
        for p in 0..0x200u32 {
            assert_eq!(operator(p, 0, 0), -operator(p + 0x200, 0, 0), "{p:#x}");
            assert_eq!(operator(p, 0, 0), operator(0x1FF - p, 0, 0), "{p:#x}");
        }
        assert!(operator(0, 0, 0).abs() <= 30);
    }

    #[test]
    fn the_key_code_follows_the_published_division_of_the_octave() {
        assert_eq!(key_code(0x100, 5), 0x14);
        assert_eq!(key_code(654, 4), 16);
        assert_eq!(key_code(925, 4), 17);
        assert_eq!(key_code(1038, 4), 18);
        assert_eq!(key_code(1165, 4), 19);
    }

    /// Writes a plain tone on channel 1 through operator 1 only (algorithm 7).
    fn tone(chip: &mut Ym2612, fnum: u16, block: u8) {
        for (offset, tl) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
            chip.write(0, 0x30 + offset, 0x01);
            chip.write(0, 0x40 + offset, tl);
            chip.write(0, 0x50 + offset, 0x1F);
            chip.write(0, 0x60 + offset, 0);
            chip.write(0, 0x70 + offset, 0);
            chip.write(0, 0x80 + offset, 0x0F);
        }
        chip.write(0, 0xB0, 0x07);
        chip.write(0, 0xB4, 0xC0);
        chip.write(0, 0xA4, block << 3 | (fnum >> 8) as u8);
        chip.write(0, 0xA0, fnum as u8);
    }

    fn run(chip: &mut Ym2612, samples: usize) -> Vec<i32> {
        (0..samples).map(|_| chip.sample().0).collect()
    }

    #[test]
    fn a_key_on_starts_a_tone_at_the_pitch_of_the_f_number() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 1083, 4);
        assert!(run(&mut chip, 200).iter().all(|v| *v == 0));
        chip.write(0, 0x28, 0x10);
        let out = run(&mut chip, 53_267);
        let crossings = out.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        // 440.1 Hz at a 7,670,454 Hz clock: 880 crossings in a second.
        assert!((crossings as i32 - 880).abs() <= 2, "{crossings}");
        assert_eq!(out.iter().max(), Some(&255));
    }

    #[test]
    fn a_key_off_releases_the_tone_to_silence() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 1083, 4);
        chip.write(0, 0x28, 0x10);
        run(&mut chip, 2000);
        chip.write(0, 0x28, 0x00);
        run(&mut chip, 20_000);
        assert!(run(&mut chip, 1000).iter().all(|v| *v == 0));
    }

    #[test]
    fn the_detune_overflow_makes_a_tone_from_an_f_number_of_zero() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 0, 0);
        chip.write(0, 0x30, 0x71);
        chip.write(0, 0x28, 0x10);
        let out = run(&mut chip, 4000);
        assert!(out.iter().any(|v| v.abs() > 10));
    }

    #[test]
    fn the_panning_bits_choose_the_side() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 1083, 4);
        chip.write(0, 0xB4, 0x80);
        chip.write(0, 0x28, 0x10);
        let mut left = 0;
        let mut right = 0;
        for _ in 0..2000 {
            let (l, r) = chip.sample();
            left = left.max(l.abs());
            right = right.max(r.abs());
        }
        assert!(left > 100);
        assert_eq!(right, 0);
    }

    #[test]
    fn the_dac_replaces_channel_six() {
        let mut chip = Ym2612::new();
        chip.write(0, 0x2B, 0x80);
        chip.write(0, 0x2A, 0xC0);
        chip.write(1, 0xB6, 0xC0);
        // Offset binary: 0xC0 is +64, doubled to 128 in the 9-bit value and
        // then scaled by DAC_GAIN / 4.
        assert_eq!(chip.sample(), (160, 160));
        chip.write(0, 0x2A, 0x00);
        assert_eq!(chip.sample(), (-320, -320));
        chip.write(0, 0x2C, 0x08);
        assert_eq!(chip.sample(), (-318, -318));
    }

    #[test]
    fn timer_a_sets_its_flag_after_the_set_number_of_samples() {
        let mut chip = Ym2612::new();
        chip.write(0, 0x24, 0xFF);
        chip.write(0, 0x25, 0x00);
        // 1020 << 2 is the register value 0x3FC: four samples to overflow.
        chip.write(0, 0x27, 0x15);
        for _ in 0..3 {
            chip.sample();
            assert_eq!(chip.status(), 0);
        }
        chip.sample();
        assert_eq!(chip.status(), STATUS_A);
        chip.write(0, 0x27, 0x15 | 0x10);
        assert_eq!(chip.status(), 0);
    }

    #[test]
    fn timer_b_counts_once_every_sixteen_samples() {
        let mut chip = Ym2612::new();
        chip.write(0, 0x26, 0xFE);
        chip.write(0, 0x27, 0x0A);
        for _ in 0..31 {
            chip.sample();
        }
        assert_eq!(chip.status(), 0);
        chip.sample();
        assert_eq!(chip.status(), STATUS_B);
    }

    #[test]
    fn the_csm_mode_keys_channel_three_on_each_timer_a_overflow() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 1083, 4);
        // Channel 3 gets the same patch through operator 1.
        for (offset, tl) in [(0u8, 0u8), (4, 127), (8, 127), (12, 127)] {
            chip.write(0, 0x32 + offset, 0x01);
            chip.write(0, 0x42 + offset, tl);
            chip.write(0, 0x52 + offset, 0x1F);
            chip.write(0, 0x82 + offset, 0x0F);
        }
        chip.write(0, 0xB2, 0x07);
        chip.write(0, 0xB6, 0xC0);
        chip.write(0, 0xA6, 4 << 3 | 4);
        chip.write(0, 0xA2, 0x3B);
        // The first operator takes its own frequency in this mode.
        chip.write(0, 0xAD, 4 << 3 | 4);
        chip.write(0, 0xA9, 0x3B);
        chip.write(0, 0x24, 0xFF);
        chip.write(0, 0x25, 0x03);
        // Mode 10 and the load bit for timer A.
        chip.write(0, 0x27, 0x80 | 0x01);
        let out: Vec<i32> = (0..400).map(|_| chip.sample().0).collect();
        assert!(out.iter().any(|v| *v != 0));
    }

    #[test]
    fn the_mask_removes_a_channel_from_the_mix_and_keeps_its_level() {
        let mut chip = Ym2612::new();
        tone(&mut chip, 1083, 4);
        chip.write(0, 0x28, 0x10);
        chip.set_mask(0x3E);
        let out = run(&mut chip, 2000);
        assert!(out.iter().all(|v| *v == 0));
        assert!(chip.levels()[0] != 0 || run(&mut chip, 200).iter().all(|v| *v == 0));
    }

    #[test]
    fn writes_to_unused_register_slots_do_nothing() {
        let mut chip = Ym2612::new();
        let before = chip.clone().sample();
        for r in [0x33u8, 0x37, 0x3B, 0x3F, 0x43, 0xB3, 0xB7] {
            chip.write(0, r, 0xFF);
        }
        assert_eq!(chip.sample(), before);
    }
}
