#!/usr/bin/env python3
"""Synthesize the original soundtrack for the Canwu promo video.

Everything is generated from code (no samples): Karplus-Strong guzheng plucks,
an additive dizi lead, string pads, synthesized taiko and drums, bells, risers,
and a convolution reverb. The arrangement follows timeline.json: 100 BPM, one
bar = 2.4 s, and the big hits land on the logo (9.6 s), the "why" climax
(98.4 s), and the outro (108 s).

Usage: python3 music.py  ->  build/music.wav (48 kHz, 16-bit stereo)
Requires numpy and scipy.
"""

import json
import pathlib
import wave

import numpy as np
from scipy.signal import butter, fftconvolve, sosfilt, sosfilt_zi

HERE = pathlib.Path(__file__).resolve().parent
TIMELINE = json.loads((HERE / "timeline.json").read_text(encoding="utf-8"))
SR = 48000
BPM = TIMELINE["bpm"]
BEAT = 60.0 / BPM
BAR = 4 * BEAT
DUR = TIMELINE["duration"]
N = int(DUR * SR) + SR * 4  # room for tails; trimmed at the end
RNG = np.random.default_rng(35)


def T(bar, beat=0.0):
    return bar * BAR + beat * BEAT


def mtof(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def tt(dur):
    return np.arange(int(dur * SR)) / SR


def noise(n):
    return RNG.uniform(-1, 1, n)


def sos(kind, freq, order=2):
    return butter(order, freq, btype=kind, fs=SR, output="sos")


def env_adsr(n, a=0.01, d=0.1, s=0.7, r=0.1, hold=None):
    """Linear ADSR over n samples; hold = time before release (default n - r)."""
    t = np.arange(n) / SR
    total = n / SR
    hold = total - r if hold is None else hold
    e = np.where(t < a, t / max(a, 1e-6), np.where(t < a + d, 1 - (1 - s) * (t - a) / max(d, 1e-6), s))
    rel = np.clip((t - hold) / max(r, 1e-6), 0, 1)
    return e * (1 - rel)


class Bus:
    def __init__(self, name):
        self.name = name
        self.x = np.zeros((2, N))

    def add(self, t0, sig, gain=1.0, pan=0.0):
        i = int(round(t0 * SR))
        if i >= N:
            return
        if sig.ndim == 1:
            ang = (pan + 1) * np.pi / 4
            sig = np.vstack([sig * np.cos(ang), sig * np.sin(ang)]) * np.sqrt(2)
        j = min(N, i + sig.shape[1])
        self.x[:, i:j] += gain * sig[:, : j - i]


# ------------------------------------------------------------------ instruments
def guzheng(freq, dur=2.6, bright=0.75, seed=0):
    """Karplus-Strong pluck, retuned exactly by resampling."""
    period = SR / freq
    p = max(2, int(round(period)))
    n = int(dur * SR * ((p + 0.5) / period)) + p + 2
    g = np.random.default_rng(seed)
    buf = g.uniform(-1, 1, p)
    for _ in range(int((1 - bright) * 6)):
        buf = 0.5 * (buf + np.roll(buf, 1))
    # out[k + 1] holds y[k]; out[0] is y[-1] = 0 so the first block has a predecessor
    out = np.zeros(n + 1)
    out[1:p + 1] = buf
    t60 = max(0.9, 3.2 - (freq - 200) / 400)
    decay = 10 ** (-3 / (freq * t60))
    i = p
    while i < n:
        j = min(i + p, n)
        out[i + 1:j + 1] = decay * 0.5 * (out[i + 1 - p:j + 1 - p] + out[i - p:j - p])
        i = j
    out = out[1:]
    # retune: the loop's period is p + 0.5 samples (the two-point average adds
    # half a sample of delay); resample so it plays at the true frequency
    src = np.arange(n)
    pos = np.arange(0, n - 1, (p + 0.5) / period)
    y = np.interp(pos, src, out)[: int(dur * SR)]
    y = sosfilt(sos("highpass", 70), y)
    y += 0.25 * sosfilt(sos("bandpass", [freq * 0.98, freq * 1.02]), y)  # body ring
    fade = np.ones_like(y)
    fade[-int(0.05 * SR):] = np.linspace(1, 0, int(0.05 * SR))
    return y * fade


def dizi(freq, dur, vib=1.0, scoop=True):
    n = int((dur + 0.18) * SR)
    t = np.arange(n) / SR
    f = np.full(n, freq)
    if scoop:
        f *= 2 ** (-0.7 / 12 * np.exp(-t / 0.035))
    vdepth = 0.007 * vib * np.clip((t - 0.22) / 0.3, 0, 1)
    f *= 1 + vdepth * np.sin(2 * np.pi * 5.3 * t)
    ph = 2 * np.pi * np.cumsum(f) / SR
    y = np.sin(ph) + 0.32 * np.sin(2 * ph) + 0.12 * np.sin(3 * ph) + 0.05 * np.sin(4 * ph)
    breath = sosfilt(sos("bandpass", [min(freq * 2.5, 9000), min(freq * 6, 15000)]), noise(n))
    e = env_adsr(n, a=0.05, d=0.15, s=0.85, r=0.16, hold=dur)
    be = env_adsr(n, a=0.02, d=0.12, s=0.35, r=0.1, hold=dur)
    return 0.6 * y * e + 0.55 * breath * be


def pad(midis, dur, cutoff=2200.0, a=1.0, r=1.6):
    n = int((dur + r) * SR)
    t = np.arange(n) / SR
    left = np.zeros(n)
    right = np.zeros(n)
    for m in midis:
        f0 = mtof(m)
        for k, det in enumerate((-9, 0, 9)):
            f = f0 * 2 ** (det / 1200)
            ph0 = RNG.uniform(0, 2 * np.pi)
            v = np.zeros(n)
            for h in range(1, 13):
                fh = f * h
                if fh > 9000:
                    break
                w = (1 / h) / np.sqrt(1 + (fh / cutoff) ** 4)
                v += w * np.sin(2 * np.pi * fh * t + ph0 * h)
            if k == 0:
                left += v
            elif k == 2:
                right += v
            else:
                left += 0.6 * v
                right += 0.6 * v
    e = env_adsr(n, a=a, d=0.5, s=0.85, r=r, hold=dur)
    sw = 1 + 0.06 * np.sin(2 * np.pi * 0.23 * t)
    return np.vstack([left * e * sw, right * e * sw]) / max(1, len(midis))


def bass(freq, dur):
    n = int((dur + 0.06) * SR)
    t = np.arange(n) / SR
    y = np.sin(2 * np.pi * freq * t) + 0.45 * np.sin(4 * np.pi * freq * t) + 0.15 * np.sin(6 * np.pi * freq * t)
    y = np.tanh(1.6 * y)
    return y * env_adsr(n, a=0.006, d=0.12, s=0.75, r=0.06, hold=dur)


def bell(freq, dur=4.0):
    t = tt(dur)
    idx = 2.6 * np.exp(-t / 0.5)
    mod = idx * np.sin(2 * np.pi * freq * 3.5 * t)
    y = np.sin(2 * np.pi * freq * t + mod) * np.exp(-t / 1.6)
    y += 0.3 * np.sin(2 * np.pi * freq * 2.76 * t) * np.exp(-t / 0.7)
    return y * np.clip(t / 0.002, 0, 1)


def kick(level=1.0):
    t = tt(0.5)
    f = 46 + 95 * np.exp(-t / 0.032)
    y = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.26)
    click = sosfilt(sos("highpass", 2000), noise(len(t))) * np.exp(-t / 0.004) * 0.35
    return np.tanh(1.4 * (y + click)) * level


def taiko(big=False):
    dur = 3.0 if big else 1.6
    t = tt(dur)
    f = (52 if big else 64) + 70 * np.exp(-t / 0.07)
    body = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / (1.0 if big else 0.45))
    skin = sosfilt(sos("lowpass", 400), noise(len(t))) * np.exp(-t / 0.09) * 1.6
    slap = sosfilt(sos("bandpass", [900, 3200]), noise(len(t))) * np.exp(-t / 0.012) * 0.5
    return np.tanh(1.3 * (body + skin + slap))


def snare():
    t = tt(0.45)
    tone = np.sin(2 * np.pi * 185 * t) * np.exp(-t / 0.05) * 0.6
    nz = sosfilt(sos("bandpass", [1400, 9000]), noise(len(t))) * np.exp(-t / 0.13) * 1.4
    return tone + nz


def clap():
    t = tt(0.5)
    nz = sosfilt(sos("bandpass", [900, 6000]), noise(len(t)))
    e = np.zeros(len(t))
    for d in (0.0, 0.012, 0.024):
        e += np.where(t >= d, np.exp(-(t - d) / 0.006), 0)
    e += np.where(t >= 0.03, 0.6 * np.exp(-(t - 0.03) / 0.11), 0)
    return nz * e * 1.2


def hat(open_=False):
    t = tt(0.4 if open_ else 0.08)
    nz = sosfilt(sos("highpass", 7500), noise(len(t)))
    return nz * np.exp(-t / (0.16 if open_ else 0.022))


def crash(dur=3.0):
    t = tt(dur)
    nz = sosfilt(sos("highpass", 3200), noise(len(t))) * np.exp(-t / 1.1)
    metal = sum(np.sin(2 * np.pi * f * t + RNG.uniform(0, 6)) for f in RNG.uniform(3000, 9000, 8)) / 8
    y = nz + 0.35 * metal * np.exp(-t / 0.7)
    return sosfilt(sos("lowpass", 12000), y) * np.clip(t / 0.003, 0, 1)


def tom(freq):
    t = tt(0.6)
    f = freq * (1 + 0.6 * np.exp(-t / 0.04))
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.22) + 0.3 * sosfilt(sos("lowpass", 900), noise(len(t))) * np.exp(-t / 0.05)


def sub_boom(dur=3.0):
    t = tt(dur)
    f = 28 + 30 * np.exp(-t / 0.5)
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 1.1) * np.clip(t / 0.005, 0, 1)


def riser(dur, lo=300, hi=9000):
    n = int(dur * SR)
    x = noise(n)
    y = np.zeros(n)
    block = 1024
    zi = None
    for i in range(0, n, block):
        k = i / n
        fc = lo * (hi / lo) ** (k ** 1.6)
        b = sos("bandpass", [fc * 0.7, min(fc * 1.4, SR / 2 - 100)])
        if zi is None or zi.shape != (b.shape[0], 2):
            zi = sosfilt_zi(b) * 0
        seg, zi = sosfilt(b, x[i:i + block], zi=zi)
        y[i:i + block] = seg
    t = np.arange(n) / SR
    sweep = np.sin(2 * np.pi * np.cumsum(220 * 2 ** (3 * t / dur)) / SR) * 0.25
    return (y + sweep) * (t / dur) ** 2.2


def swell(dur):
    c = crash(dur + 0.2)[::-1][-int(dur * SR):]
    return c * np.linspace(0, 1, len(c)) ** 2


def wind(dur):
    n = int(dur * SR)
    t = np.arange(n) / SR
    y = sosfilt(sos("bandpass", [250, 1400]), noise(n))
    lfo = 0.6 + 0.4 * np.sin(2 * np.pi * 0.13 * t) * np.sin(2 * np.pi * 0.07 * t + 1)
    return y * lfo


# ------------------------------------------------------------------ harmony
CHORDS = {
    "Bm": (35, [54, 59, 62, 66, 73], [59, 62, 66, 71, 74, 71, 66, 62]),
    "G": (31, [55, 59, 62, 67, 69], [55, 59, 62, 67, 71, 67, 62, 59]),
    "D": (38, [54, 57, 62, 66, 69], [57, 62, 66, 69, 74, 69, 66, 62]),
    "A": (33, [52, 57, 61, 64, 71], [57, 64, 69, 71, 76, 71, 69, 64]),
}
PROG = (["Bm", "Bm", "G", "A"] + ["Bm", "G"] + ["Bm", "G", "D", "A"] + ["Bm", "G", "D", "A"] * 6
        + ["Bm", "G", "D", "A"] + ["Bm", "G", "A"] + ["G", "D", "A", "Bm"] + ["D", "G", "D", "D", "D"])
assert len(PROG) == 50, len(PROG)

PHRASE_A = [(0, 66, 1), (1, 69, .5), (1.5, 71, .5), (2, 74, 1.5), (3.5, 71, .5),
            (4, 69, 1), (5, 71, .5), (5.5, 69, .5), (6, 66, 1.5), (7.5, 64, .5),
            (8, 66, .5), (8.5, 69, .5), (9, 74, 1.5), (10.5, 76, .5), (11, 78, 1),
            (12, 76, 1.5), (13.5, 74, .5), (14, 71, .5), (14.5, 69, 1.5)]
PHRASE_B = [(0, 71, .75), (.75, 74, .25), (1, 78, 1), (2, 76, .5), (2.5, 74, .5), (3, 71, 1),
            (4, 74, 1), (5, 71, .5), (5.5, 69, .5), (6, 71, 2),
            (8, 69, .5), (8.5, 71, .5), (9, 74, .5), (9.5, 76, .5), (10, 78, 1), (11, 81, 1),
            (12, 76, 1), (13, 78, .5), (13.5, 76, .5), (14, 76, 2)]
PHRASE_C = [(0, 74, 1), (1, 78, 1), (2, 81, 1.5), (3.5, 78, .5),
            (4, 78, 1), (5, 76, .5), (5.5, 74, .5), (6, 74, 1.5), (7.5, 76, .5),
            (8, 76, 1), (9, 78, .5), (9.5, 76, .5), (10, 71, 1), (11, 69, 1),
            (12, 71, 1.5), (13.5, 74, .5), (14, 78, 2)]
INTRO = [(0, 71, 1.5), (1.5, 78, .5), (2, 76, 1), (3, 74, 1),
         (4, 71, 3), (7, 69, .5), (7.5, 71, .5),
         (8, 74, 1), (9, 76, .5), (9.5, 74, .5), (10, 71, 2),
         (12, 69, 2), (14, 66, 1), (15, 64, 1)]
LOGO = [(0, 78, 2), (2, 76, 1), (3, 74, 1), (4, 71, 4)]
OUTRO = [(0, 74, 2), (2, 78, 1), (3, 81, 1), (4, 83, 2), (6, 81, 1), (7, 78, 1), (8, 74, 6)]
PENTA = [47, 50, 52, 54, 57, 59, 62, 64, 66, 69, 71, 74, 76, 78, 81, 83, 86]


def main():
    drums, bassb, padb, arps, lead, zheng, fx, amb = (Bus(n) for n in
                                                      ("drums", "bass", "pad", "arps", "lead", "zheng", "fx", "amb"))
    kicks = []

    def K(t, lvl=1.0):
        drums.add(t, kick(lvl))
        kicks.append(t)

    def play_zheng(notes, bar0, gain=1.0, octave=0, pan=0.2, dur=2.8, seed=0):
        for k, (b, m, _) in enumerate(notes):
            zheng.add(T(bar0, b), guzheng(mtof(m + octave), dur, seed=seed + k), gain, pan)

    def play_dizi(notes, bar0, gain=1.0, octave=0, pan=-0.05):
        for b, m, d in notes:
            lead.add(T(bar0, b), dizi(mtof(m + octave), d * BEAT * 0.98), gain, pan)

    def gliss(t0, up=True, span=0.45, lo=0, hi=None, gain=0.6):
        notes = PENTA[lo:hi]
        if not up:
            notes = notes[::-1]
        for k, m in enumerate(notes):
            zheng.add(t0 + span * k / len(notes), guzheng(mtof(m), 2.0, seed=500 + k), gain * (0.7 + 0.3 * k / len(notes)), -0.4 + 0.8 * k / len(notes))

    def hit(t0, chord):
        drums.add(t0, taiko(big=True), 1.0)
        drums.add(t0, sub_boom(), 0.9)
        fx.add(t0, crash(4.0), 0.55, 0.25)
        for k, m in enumerate(CHORDS[chord][1][1:4]):
            fx.add(t0 + 0.02 * k, bell(mtof(m + 12)), 0.25, -0.3 + 0.3 * k)

    # --- ambience and pads -------------------------------------------------
    amb.add(0, wind(11.0) * np.clip(tt(11.0) / 2.5, 0, 1) * np.clip((11.0 - tt(11.0)) / 2.0, 0, 1), 1.0)
    amb.add(T(45), wind(10.0) * np.clip(tt(10.0) / 2.0, 0, 1) * np.clip((10.0 - tt(10.0)) / 4.0, 0, 1), 0.8)
    for bar, ch in enumerate(PROG):
        root, voicing, arp = CHORDS[ch]
        cutoff = 1300 if bar < 4 else 1900 if bar < 10 else 2600 if bar < 41 else 3600
        if bar >= 45:
            cutoff = 2400
        dur = BAR + (BAR * 2 if bar == 49 else 0.05)
        padb.add(T(bar), pad(voicing, dur, cutoff=cutoff, a=0.5 if bar in (4, 45) else 0.9), 1.0)

        # bass
        if bar < 4 or bar >= 45:
            if bar >= 2:
                bassb.add(T(bar), bass(mtof(root), BAR * 0.95), 0.8)
        elif bar < 6:
            bassb.add(T(bar), bass(mtof(root), BAR * 0.95), 0.9)
        elif bar in (34, 35, 36, 37, 38, 39):
            for b in (0, 2.5):
                bassb.add(T(bar, b), bass(mtof(root), BEAT * 1.4), 0.85)
        else:
            pattern = [0, 0, 12, 0, 0, 0, 12, 7]
            for k, off in enumerate(pattern):
                bassb.add(T(bar, k * 0.5), bass(mtof(root + off), BEAT * 0.45), 0.85 if k % 2 == 0 else 0.7)

        # arpeggios
        if 6 <= bar < 10:
            for k in range(8):
                arps.add(T(bar, k * 0.5), guzheng(mtof(arp[k]), 1.2, bright=0.6, seed=bar * 16 + k), 0.55, 0.35 if k % 2 else -0.35)
        elif 10 <= bar < 45 and bar != 40:
            for k in range(16):
                arps.add(T(bar, k * 0.25), guzheng(mtof(arp[k % 8]), 0.9, bright=0.55, seed=bar * 16 + k),
                         0.45 if k % 4 == 0 else 0.3, 0.4 if k % 2 else -0.4)

    # --- intro -------------------------------------------------------------
    gliss(0.15, up=True, span=0.6, lo=4, hi=14, gain=0.45)
    play_zheng(INTRO, 0, gain=0.9, seed=10)
    for t0, lvl in ((T(1), 0.35), (T(2), 0.45), (T(3), 0.6), (T(3, 2), 0.7)):
        drums.add(t0, taiko(), lvl)
    for k in range(8):  # tom build in the last beat before the logo hit
        drums.add(T(3, 2) + k * BEAT / 4, tom(110 + 12 * k), 0.25 + 0.06 * k)
    fx.add(T(2), riser(BAR * 2), 0.45)
    fx.add(T(3), swell(BAR), 0.5)

    # --- logo hit ------------------------------------------------------------
    hit(T(4), "Bm")
    gliss(T(4), up=True, span=0.5, lo=5, hi=16, gain=0.55)
    play_zheng(LOGO, 4, gain=0.9, seed=40)
    for k, m in enumerate([74, 78, 81, 83, 86, 83, 81, 78]):
        fx.add(T(4, 1 + k * 0.5), bell(mtof(m)), 0.12, -0.5 + k * 0.14)
    drums.add(T(5), taiko(), 0.5)

    # --- "what" section: pulse builds -----------------------------------------
    for bar in range(6, 10):
        for b in (0, 2):
            K(T(bar, b), 0.75)
        for k in range(16):
            drums.add(T(bar, k * 0.25), hat(), 0.18 if k % 2 else 0.1, 0.3)
        if bar >= 8:
            for b in (1, 3):
                drums.add(T(bar, b), clap(), 0.4, -0.1)
    fx.add(T(8), riser(BAR * 2), 0.4)
    for k in range(8):
        drums.add(T(9, 2) + k * BEAT / 4, snare(), 0.15 + 0.05 * k, 0.1)

    # --- features (bars 10-33) ------------------------------------------------
    for bar in range(10, 34):
        sec = (bar - 10) // 4
        inbar = (bar - 10) % 4
        breakdown = bar >= 30
        if inbar == 0:
            fx.add(T(bar), crash(), 0.4, 0.3 if sec % 2 else -0.3)
            drums.add(T(bar), taiko(), 0.6)
        for s16 in (0, 6, 8) if not breakdown else (0, 8):
            K(T(bar, s16 / 4), 0.95 if not breakdown else 0.8)
        for b in (1, 3):
            drums.add(T(bar, b), clap(), 0.55 if not breakdown else 0.4, -0.1)
            drums.add(T(bar, b), snare(), 0.3 if not breakdown else 0.2, 0.1)
        dense = 18 <= bar < 30
        for k in range(16):
            if k % 2 == 0:
                drums.add(T(bar, k * 0.25), hat(), 0.2 if k % 4 == 2 else 0.12, 0.35)
            elif dense:
                drums.add(T(bar, k * 0.25), hat(), 0.07, 0.35)
        if inbar == 3 and bar < 33:
            for k, f in enumerate((180, 160, 140, 120)):
                drums.add(T(bar, 3 + k * 0.25), tom(f), 0.5, -0.4 + 0.25 * k)
    play_dizi(PHRASE_A, 10, gain=0.85)
    play_dizi(PHRASE_B, 14, gain=0.85)
    play_dizi(PHRASE_A, 18, gain=0.85, octave=0)
    play_zheng(PHRASE_A, 18, gain=0.35, octave=-12, seed=200)
    play_dizi(PHRASE_B, 22, gain=0.9)
    play_zheng(PHRASE_A, 26, gain=0.85, seed=300)
    for k in range(4):  # agents: soft bell motif over the breakdown
        fx.add(T(30 + k), bell(mtof([78, 76, 74, 76][k])), 0.12, 0.3)
    fx.add(T(32), riser(BAR * 2), 0.3)

    # --- get started + skills (bars 34-40): half-time --------------------------
    for bar in range(34, 41):
        if bar == 34:
            fx.add(T(bar), crash(), 0.35, -0.3)
        if bar < 40:
            K(T(bar, 0), 0.85)
            K(T(bar, 2.5), 0.7)
            drums.add(T(bar, 2), clap(), 0.5, -0.1)
            drums.add(T(bar, 2), snare(), 0.25, 0.1)
            for k in range(8):
                drums.add(T(bar, k * 0.5), hat(), 0.16 if k % 2 else 0.09, 0.35)
    play_zheng([(0, 71, 1), (1, 74, 1), (2, 76, 2), (4, 74, 1), (5, 71, 1), (6, 69, 2)], 36, gain=0.6, seed=400)
    play_zheng([(0, 71, 1), (1, 74, 1), (2, 78, 2), (4, 76, 2), (6, 74, 2)], 38, gain=0.6, seed=420)
    # build into the climax
    fx.add(T(39), riser(BAR * 2), 0.5)
    fx.add(T(40), swell(BAR), 0.45)
    for k in range(16):
        drums.add(T(40, k * 0.25), snare(), 0.1 + 0.03 * k, 0.1)
    for k in range(4):
        drums.add(T(40, 3 + k * 0.25), tom(170 - 15 * k), 0.55)
    padb.add(T(40), pad([57, 61, 64, 69, 76], BAR, cutoff=3000), 0.6)

    # --- why: climax (bars 41-44) ---------------------------------------------
    hit(T(41), "G")
    gliss(T(41), up=True, span=0.4, lo=6, hi=17, gain=0.5)
    for bar in range(41, 45):
        if bar > 41:
            fx.add(T(bar), crash(), 0.28, 0.3)
        for s16 in (0, 6, 8, 14):
            K(T(bar, s16 / 4), 1.0)
        for b in (1, 3):
            drums.add(T(bar, b), clap(), 0.6, -0.1)
            drums.add(T(bar, b), snare(), 0.35, 0.1)
        for b in (0, 2):
            drums.add(T(bar, b), taiko(), 0.55)
        for k in range(16):
            drums.add(T(bar, k * 0.25), hat(open_=(k % 4 == 2)), 0.2 if k % 2 == 0 else 0.08, 0.35)
    play_dizi(PHRASE_C, 41, gain=0.95)
    play_zheng(PHRASE_C, 41, gain=0.55, octave=-12, seed=600)
    fx.add(T(44), riser(BAR), 0.45)
    fx.add(T(44, 2), swell(BAR / 2), 0.5)

    # --- outro (bars 45-49) ------------------------------------------------------
    hit(T(45), "D")
    gliss(T(45), up=False, span=0.7, lo=4, hi=17, gain=0.5)
    play_zheng(OUTRO, 45, gain=0.85, seed=700)
    drums.add(T(47), taiko(), 0.35)
    for k, m in enumerate([86, 81, 78, 74]):
        fx.add(T(47, 2 + k * 0.5), bell(mtof(m)), 0.1, 0.4 - 0.25 * k)

    # --- mix --------------------------------------------------------------------
    duck = np.ones(N)
    for t0 in kicks:
        i = int(t0 * SR)
        seg = 1 - 0.32 * np.exp(-np.arange(int(0.4 * SR)) / (0.12 * SR))
        j = min(N, i + len(seg))
        duck[i:j] = np.minimum(duck[i:j], seg[: j - i])
    padb.x *= duck
    arps.x *= duck

    def active_rms(x):
        mono = x.mean(axis=0)
        frame = 4800
        k = len(mono) // frame
        fr = np.sqrt((mono[: k * frame].reshape(k, frame) ** 2).mean(axis=1))
        act = fr[fr > fr.max() * 0.03]
        return float(np.sqrt((act ** 2).mean())) if len(act) else 1.0

    # relative bus levels in dB
    levels = {"drums": 0.0, "bass": -4.0, "pad": -8.5, "arps": -14.0, "lead": -4.5, "zheng": -5.5, "fx": -9.0, "amb": -21.0}
    sends = {"drums": 0.08, "bass": 0.0, "pad": 0.35, "arps": 0.3, "lead": 0.3, "zheng": 0.4, "fx": 0.45, "amb": 0.2}
    buses = [drums, bassb, padb, arps, lead, zheng, fx, amb]
    ref = active_rms(drums.x)
    dry = np.zeros((2, N))
    send = np.zeros((2, N))
    for b in buses:
        g = ref / active_rms(b.x) * 10 ** (levels[b.name] / 20)
        dry += b.x * g
        send += b.x * g * sends[b.name]

    # convolution reverb: decaying stereo noise with early reflections
    ir_t = tt(3.2)
    ir = np.vstack([noise(len(ir_t)), noise(len(ir_t))]) * np.exp(-ir_t / 0.62)
    ir = sosfilt(sos("lowpass", 6500), ir, axis=1)
    for d, g in ((0.011, 0.5), (0.023, 0.35), (0.037, 0.3)):
        ir[:, int(d * SR)] += g
    ir /= np.sqrt((ir ** 2).sum(axis=1, keepdims=True))
    send = sosfilt(sos("highpass", 180), send, axis=1)
    wet = np.vstack([fftconvolve(send[c], ir[c])[:N] for c in range(2)])
    mix = dry + 0.55 * wet
    mix = sosfilt(sos("highpass", 28), mix, axis=1)

    # master: section dynamics, fades, and limiter-style soft clip
    total = int(DUR * SR)
    mix = mix[:, :total]
    curve = [(0, -7), (6.0, -5.5), (9.3, -3), (9.6, 0), (12.0, -2.5), (14.4, -3.5), (23.4, -2.5), (24.0, -0.5),
             (71.8, -0.5), (72.0, -2.0), (81.6, -3.0), (96.0, -3.0), (98.3, -1.0), (98.4, 0), (107.9, 0), (108.0, 0.5), (DUR, 0)]
    secs = np.arange(total) / SR
    mix *= 10 ** (np.interp(secs, [c[0] for c in curve], [c[1] for c in curve]) / 20)
    fade_in = np.clip(np.arange(total) / (0.05 * SR), 0, 1)
    tail = np.clip((DUR - np.arange(total) / SR) / 3.2, 0, 1) ** 1.5
    mix *= fade_in * tail
    mix /= np.abs(mix).max()
    mix = np.tanh(1.5 * mix) / np.tanh(1.5)
    mix *= 10 ** (-1.0 / 20) / np.abs(mix).max()

    out = HERE / "build"
    out.mkdir(exist_ok=True)
    pcm = (mix.T * 32767).astype("<i2")
    with wave.open(str(out / "music.wav"), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    print(f"wrote build/music.wav ({total / SR:.1f} s)")


if __name__ == "__main__":
    main()
