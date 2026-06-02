---
name: Bug report
about: You hit something that didn't work. Let's fix it.
title: ''
labels: bug
---

<!--
The most useful bug reports come from someone who actually hit the
problem and wants it fixed. Skip whatever section doesn't apply,
but the more concrete this is, the faster a fix can land.
-->

## What happened

A short, factual description of the symptom. "Audio drops out
after ~10 seconds of holding a chord" beats "audio is broken."

## What you expected to happen

What did you think should have happened instead?

## How to reproduce

The exact steps that trigger this. Patch / preset / script content
if relevant. A small, focused reproduction is the most useful kind
of report.

## Environment

- **Brume version:** (footer status bar, SYS page, or `brumectl --version`)
- **Hardware:** CM5 + which IO board + which screen, or "Linux desktop" with distribution + kernel
- **Anything else relevant:** non-default audio HAT, custom controller, etc.

## Logs / screenshots

If relevant: `journalctl --user -u brume.service` on the CM5, or
`/tmp/brume.log` if running ad-hoc. A short video or screenshot
helps for UI-shaped bugs.

## Anything else

Theories, related observations, things you tried that didn't help.
