#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright (C) 2026 Keir Finlow-Bates
"""Makes the mobs' placeholder sounds (media/mobs_*.ogg): mono Ogg Vorbis
at 22.05 kHz, drawn from noise and tones. Needs numpy and ffmpeg.

    python3 tools/make_mob_sounds.py           the mossling's (not its call)
    python3 tools/make_mob_sounds.py --call    and its call
    python3 tools/make_mob_sounds.py --wisp    the wisp's
    python3 tools/make_mob_sounds.py --hexer   the hexer's
"""
import os
import subprocess
import tempfile
import wave

import numpy as np

RATE = 22050
MEDIA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media")
rng = np.random.default_rng(7)


def t(secs):
    return np.arange(int(RATE * secs)) / RATE


def env(n, attack=0.01, release=0.1):
    e = np.ones(n)
    a, r = int(RATE * attack), int(RATE * release)
    if a:
        e[:a] = np.linspace(0, 1, a)
    if r:
        e[-r:] *= np.linspace(1, 0, r)
    return e


def tone(freqs, harmonics=(1.0, 0.5, 0.25), vibrato=0.0, vib_rate=7.0):
    """A voice whose pitch follows `freqs` (one per sample)."""
    phase = 2 * np.pi * np.cumsum(freqs * (1 + vibrato * np.sin(2 * np.pi * vib_rate * np.arange(len(freqs)) / RATE))) / RATE
    return sum(a * np.sin((k + 1) * phase) for k, a in enumerate(harmonics))


def lowpass(x, alpha):
    y = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc += alpha * (v - acc)
        y[i] = acc
    return y


def bandnoise(n, lo_alpha, hi_alpha):
    w = rng.standard_normal(n)
    return lowpass(w, hi_alpha) - lowpass(w, lo_alpha)


def growl(x, depth=0.5, rate=35.0):
    return x * (1 - depth + depth * np.abs(np.sin(np.pi * rate * np.arange(len(x)) / RATE)))


def snap(n_total, at, strength=1.0):
    out = np.zeros(n_total)
    i = int(at * RATE)
    k = min(int(0.02 * RATE), n_total - i)
    if k > 0:
        burst = rng.standard_normal(k) * np.exp(-np.linspace(0, 8, k)) * strength
        out[i:i + k] += burst - lowpass(burst, 0.3)
    return out


def voice(f, growl_rate=40.0, breath=0.12, vibrato=0.04, vib_rate=9.0):
    """A croaky voice following the pitch curve `f`, like the call: rich in
    harmonics, rough, and a little breathy, so it never sounds like a beep."""
    n = len(f)
    v = growl(tone(f, (1.0, 0.7, 0.45, 0.3, 0.15), vibrato=vibrato, vib_rate=vib_rate), 0.45, growl_rate)
    return v + bandnoise(n, 0.02, 0.4) * breath * 8


def arc(n, lo, hi, end=None, power=1.5):
    """A pitch curve rising from `lo` to `hi` and falling to `end`."""
    x = np.linspace(0, 1, n)
    end = lo if end is None else end
    return lo + (end - lo) * x + (hi - (lo + end) / 2) * np.sin(np.pi * x) ** power


def chatter(seed):
    """Two or three short croaks, little cousins of the call, with a twig
    snap or two."""
    r = np.random.default_rng(seed)
    syllables = r.integers(2, 4)
    out = np.zeros(0)
    for _ in range(syllables):
        n = len(t(r.uniform(0.12, 0.22)))
        lo = r.uniform(170, 230)
        f = arc(n, lo, r.uniform(280, 380), lo * r.uniform(0.85, 1.1))
        syl = voice(f, r.uniform(30, 45), 0.1, 0.05, r.uniform(7, 11)) * env(n, 0.03, 0.08) * 0.6
        gap = np.zeros(len(t(r.uniform(0.04, 0.1))))
        out = np.concatenate([out, syl, gap])
    for _ in range(r.integers(1, 3)):
        out += snap(len(out), r.uniform(0.0, len(out) / RATE - 0.05), 0.4)
    return out


def call():
    n = len(t(0.9))
    x = np.linspace(0, 1, n)
    f = 190 + 260 * np.sin(np.pi * x) ** 1.5
    v = growl(tone(f, (1.0, 0.7, 0.45, 0.3, 0.15), vibrato=0.04, vib_rate=9), 0.45, 40)
    return v * env(n, 0.05, 0.25) * 0.6 + bandnoise(n, 0.02, 0.4) * env(n, 0.05, 0.3) * 0.08


def rustle(n, amount=1.0):
    """Leaves and twigs: bursts of soft noise, fluttering."""
    w = bandnoise(n, 0.08, 0.6)
    flutter = np.abs(lowpass(rng.standard_normal(n), 0.004)) * 6
    return w * flutter * amount


def swipe():
    """A rustling swish of leafy arms, ending in a snap."""
    n = len(t(0.35))
    x = np.linspace(0, 1, n)
    w = rng.standard_normal(n)
    alpha = 0.04 + 0.3 * np.sin(np.pi * x) ** 2
    y = np.empty(n)
    acc = 0.0
    for i in range(n):
        acc += alpha[i] * (w[i] - acc)
        y[i] = acc
    swish = (y - lowpass(y, 0.02)) * np.sin(np.pi * x) ** 1.5
    return swish * 1.2 + rustle(n, 0.3) * np.sin(np.pi * x) + snap(n, 0.22, 0.5)


def hurt():
    """A short, higher yelp: the call, squeezed."""
    n = len(t(0.32))
    f = arc(n, 300, 620, 340, 1.0)
    return voice(f, 55, 0.12, 0.06, 13) * env(n, 0.01, 0.15) * 0.6 + rustle(n, 0.2)


def death():
    """A long croak sinking away into a gurgle, and a crumble of twigs."""
    n = len(t(1.1))
    x = np.linspace(0, 1, n)
    f = arc(n, 260, 420, 110, 1.0) * (1 - 0.15 * x)
    v = voice(f, 30, 0.1, 0.03 + 0.12 * x, 6) * env(n, 0.03, 0.5) * 0.6
    crackle = sum(snap(n, a, 0.45) for a in (0.55, 0.68, 0.74, 0.86, 0.93))
    return v + crackle + rustle(n, 0.25) * x


def seamless(x, fade=0.15):
    """Crossfade a loop's end into its start, so it loops without a click."""
    k = int(fade * RATE)
    head, tail = x[:k].copy(), x[-k:].copy()
    ramp = np.linspace(0, 1, k)
    x = x[:-k].copy()
    x[:k] = head * ramp + tail * (1 - ramp)
    return x


def flame(n, brightness=0.35):
    """A soft roar of flame: low noise, gently fluttering."""
    w = lowpass(rng.standard_normal(n), brightness * 0.3) * 3
    flutter = 0.6 + 0.4 * np.abs(lowpass(rng.standard_normal(n), 0.002)) * 8
    return w * np.clip(flutter, 0, 1.6)


def wisp_hum():
    """The wisp's hum (looped): a breathy, wavering tone over a soft flame,
    a little eerie."""
    n = len(t(2.15))
    x = np.arange(n) / RATE
    f = 155 + 6 * np.sin(2 * np.pi * 0.5 * x) + 3 * np.sin(2 * np.pi * 3 * x)
    v = tone(f, (1.0, 0.25, 0.35, 0.1), vibrato=0.01, vib_rate=5)
    shimmer = tone(f * 2.01, (0.3, 0.1)) * (0.5 + 0.5 * np.sin(2 * np.pi * 1.5 * x))
    return seamless(v * 0.5 + shimmer * 0.25 + flame(n) * 0.5)


def wisp_whine():
    """The fuse: 1.5 s, rising in pitch and loudness, flickering faster,
    crackling more, to the burst."""
    n = len(t(1.5))
    x = np.linspace(0, 1, n)
    f = 260 * (1 + 3.2 * x ** 1.6)
    flicker_rate = 6 + 30 * x
    flicker = 0.65 + 0.35 * np.sin(2 * np.pi * np.cumsum(flicker_rate) / RATE)
    v = tone(f, (1.0, 0.5, 0.3, 0.2), vibrato=0.02, vib_rate=11) * flicker
    crackle = sum(snap(n, a, 0.3 + 0.5 * a / 1.5) for a in np.sort(rng.uniform(0.1, 1.45, 14)))
    return (v * 0.6 + flame(n, 0.6) * 0.3 + crackle) * (0.3 + 0.7 * x) * env(n, 0.05, 0.02)


def wisp_fizzle():
    """Put out: a hiss falling away, with a fading wobble of the hum."""
    n = len(t(0.7))
    x = np.linspace(0, 1, n)
    hiss = (rng.standard_normal(n) - lowpass(rng.standard_normal(n), 0.2)) * np.exp(-4 * x)
    f = 300 * (1 - 0.6 * x)
    v = tone(f, (1.0, 0.3)) * np.exp(-5 * x)
    return hiss * 0.5 + v * 0.4 + snap(n, 0.0, 0.5)


def resonate(x, freqs, q=8.0):
    """A band-pass resonator whose centre follows `freqs` (one per sample):
    shapes noise into vowel-like whispers."""
    y = np.zeros_like(x)
    y1 = y2 = 0.0
    for i in range(len(x)):
        w = 2 * np.pi * freqs[i] / RATE
        r = 1 - w / (2 * q)
        a1, a2 = 2 * r * np.cos(w), -r * r
        v = x[i] * (1 - r) + a1 * y1 + a2 * y2
        y2, y1 = y1, v
        y[i] = v
    return y


def whisper(secs, seed, base=1.0):
    """Breathy, wordless whispering: syllables of noise through two moving
    formants, the vowels changing at random."""
    r = np.random.default_rng(seed)
    n = len(t(secs))
    w = r.standard_normal(n)
    f1, f2, amp = np.zeros(n), np.zeros(n), np.zeros(n)
    i = 0
    while i < n:
        m = int(r.uniform(0.08, 0.2) * RATE)
        j = min(n, i + m)
        f1[i:j] = np.linspace(r.uniform(300, 800), r.uniform(300, 800), j - i) * base
        f2[i:j] = np.linspace(r.uniform(900, 2400), r.uniform(900, 2400), j - i) * base
        amp[i:j] = np.sin(np.linspace(0, np.pi, j - i)) ** 0.7 * (0 if r.random() < 0.15 else 1)
        i = j + int(r.uniform(0.0, 0.05) * RATE)
    f1, f2 = lowpass(f1, 0.01), lowpass(f2, 0.01)
    v = resonate(w, f1, 6) * 0.8 + resonate(w, f2, 9) * 0.5
    hiss = (w - lowpass(w, 0.5)) * 0.15
    return (v + hiss) * lowpass(amp, 0.01)


def drone(n, f0, rise=1.0):
    """A low, dark drone, its pitch moving from f0 to f0 * rise."""
    f = np.linspace(f0, f0 * rise, n)
    return tone(f, (1.0, 0.6, 0.5, 0.35, 0.3, 0.2), vibrato=0.01, vib_rate=4)


def hexer_mutter(seed):
    n = len(t(1.8))
    return whisper(1.8, seed, 0.8) * env(n, 0.1, 0.3) + drone(n, 55) * 0.06 * env(n, 0.3, 0.5)


def hexer_hiss():
    n = len(t(0.9))
    x = np.linspace(0, 1, n)
    w = rng.standard_normal(n)
    hiss = resonate(w, np.linspace(2600, 3600, n), 3) * np.sin(np.pi * x ** 0.6) ** 2
    return hiss * 1.2 + growl(drone(n, 70, 0.8), 0.6, 25) * 0.2 * env(n, 0.1, 0.3)


def hexer_cast():
    """Drawing the life in: a dark swell, rushing in, rising, ending in a
    soft release."""
    n = len(t(0.8))
    x = np.linspace(0, 1, n)
    rush = resonate(rng.standard_normal(n), 300 + 1800 * x ** 2, 2.5) * x ** 2
    d = drone(n, 65, 2.2) * x ** 1.5
    return rush * 0.9 + d * 0.35 + snap(n, 0.76, 0.6)


def hexer_orb():
    """An orb in flight (looped): a low pulsing throb, sizzling."""
    n = len(t(1.2))
    x = np.arange(n) / RATE
    pulse = 0.6 + 0.4 * np.sin(2 * np.pi * 5 * x)
    d = tone(np.full(n, 95.0), (1.0, 0.5, 0.4, 0.25)) * pulse
    sizzle = (rng.standard_normal(n) - lowpass(rng.standard_normal(n), 0.3)) * 0.12
    return seamless(d * 0.5 + sizzle + flame(n, 0.4) * 0.3, 0.1)


def hexer_impact():
    n = len(t(0.7))
    x = np.linspace(0, 1, n)
    thump = tone(np.linspace(110, 45, n), (1.0, 0.5)) * np.exp(-9 * x)
    sizzle = (rng.standard_normal(n) - lowpass(rng.standard_normal(n), 0.25)) * np.exp(-4 * x) * 0.5
    crackle = sum(snap(n, a, 0.4) for a in rng.uniform(0.05, 0.5, 6))
    return thump * 0.9 + sizzle + crackle


def hexer_blink():
    """Folding out of the air: a quick sucked-in whoosh and a pop."""
    n = len(t(0.35))
    x = np.linspace(0, 1, n)
    whoosh = resonate(rng.standard_normal(n), 2400 * (1 - x) + 200, 2.5) * np.sin(np.pi * x) ** 2
    return whoosh * 1.1 + snap(n, 0.3, 0.8)


def hexer_hurt():
    n = len(t(0.4))
    w = rng.standard_normal(n)
    rasp = growl(resonate(w, np.linspace(900, 600, n), 4), 0.7, 45)
    return rasp * env(n, 0.01, 0.2) * 1.2 + drone(n, 90, 0.7) * 0.15 * env(n, 0.01, 0.2)


def hexer_death():
    """A long rattling breath out, falling, the whisper dying away."""
    n = len(t(1.6))
    x = np.linspace(0, 1, n)
    w = rng.standard_normal(n)
    breath = growl(resonate(w, 1400 * (1 - 0.6 * x), 3), 0.5 + 0.4 * x, 18) * (1 - x) ** 1.2
    return breath * 1.1 + drone(n, 80, 0.5) * 0.25 * (1 - x) + whisper(1.6, 9, 0.6) * 0.3 * (1 - x)


def hexer_vanish():
    """Gone in smoke: a soft rush of air falling away, a last whisper."""
    n = len(t(0.9))
    x = np.linspace(0, 1, n)
    rush = resonate(rng.standard_normal(n), 1600 * (1 - x) + 300, 2) * np.sin(np.pi * x ** 0.5) * (1 - x)
    return rush * 1.2 + whisper(0.9, 12, 1.1) * 0.25 * (1 - x)


def save(name, samples):
    samples = samples / max(1e-6, np.max(np.abs(samples))) * 0.85
    pcm = (samples * 32767).astype("<i2")
    with tempfile.TemporaryDirectory() as tmp:
        wav = os.path.join(tmp, "s.wav")
        with wave.open(wav, "wb") as f:
            f.setnchannels(1)
            f.setsampwidth(2)
            f.setframerate(RATE)
            f.writeframes(pcm.tobytes())
        out = os.path.join(MEDIA, name)
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", wav, "-c:a", "libvorbis", "-q:a", "1", "-ac", "1", "-ar", str(RATE), "-map_metadata", "-1", out], check=True)
        print(name, os.path.getsize(out))


if __name__ == "__main__":
    import sys
    if "--hexer" in sys.argv:
        for i in (1, 2):
            save(f"mobs_hexer_mutter.{i}.ogg", hexer_mutter(200 + i))
        save("mobs_hexer_hiss.ogg", hexer_hiss())
        save("mobs_hexer_cast.ogg", hexer_cast())
        save("mobs_hexer_orb.ogg", hexer_orb())
        save("mobs_hexer_impact.ogg", hexer_impact())
        save("mobs_hexer_blink.ogg", hexer_blink())
        save("mobs_hexer_hurt.ogg", hexer_hurt())
        save("mobs_hexer_death.ogg", hexer_death())
        save("mobs_hexer_vanish.ogg", hexer_vanish())
        sys.exit()
    if "--wisp" in sys.argv:
        save("mobs_wisp_hum.ogg", wisp_hum())
        save("mobs_wisp_whine.ogg", wisp_whine())
        save("mobs_wisp_fizzle.ogg", wisp_fizzle())
        sys.exit()
    for i in (1, 2, 3):
        save(f"mobs_mossling_chatter.{i}.ogg", chatter(100 + i))
    # The call is kept as first made (liked as it is); remake it with
    # --call.
    if "--call" in __import__("sys").argv:
        save("mobs_mossling_call.ogg", call())
    save("mobs_mossling_swipe.ogg", swipe())
    save("mobs_mossling_hurt.ogg", hurt())
    save("mobs_mossling_death.ogg", death())
