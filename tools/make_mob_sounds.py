#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright (C) 2026 Keir Finlow-Bates
"""Makes the mobs' placeholder sounds (media/mobs_*.ogg): mono Ogg Vorbis
at 22.05 kHz, drawn from noise and tones. Needs numpy and ffmpeg.

    python3 tools/make_mob_sounds.py
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
    for i in (1, 2, 3):
        save(f"mobs_mossling_chatter.{i}.ogg", chatter(100 + i))
    # The call is kept as first made (liked as it is); remake it with
    # --call.
    if "--call" in __import__("sys").argv:
        save("mobs_mossling_call.ogg", call())
    save("mobs_mossling_swipe.ogg", swipe())
    save("mobs_mossling_hurt.ogg", hurt())
    save("mobs_mossling_death.ogg", death())
