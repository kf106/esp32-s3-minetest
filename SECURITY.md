# Security Policy

## Supported versions

Security fixes go into the latest release only (or, until the first release, the latest commit on the `main` branch). See the [releases page](https://github.com/kf106/esp32-s3-minetest/releases).

A fix may go into the next planned release or, if it is serious, into a patch release as soon as possible. Advisories are published once a release with the fix is out.

## Scope

In scope is anything that lets a client, or anyone else on the network, compromise the confidentiality, integrity or availability of the server or of other players. For example:

* crashing, hanging or rebooting the server with malformed or unexpected packets
* corrupting or deleting the world data on the SD card or in flash
* logging in as another player without their password, reading other players' credentials, or gaining privileges that were not granted
* running code on the device

It also covers the server taking over or crashing a standard Luanti client.

The following are treated as ordinary bugs, not vulnerabilities:

* someone with physical access to the board or SD card reading, changing or reflashing it
* the operator misconfiguring the server so that it crashes or stops working
* the network protocol being unencrypted (it is Luanti's protocol)
* a client sending false data for an in-game advantage (such as movement cheats)
* the server slowing down or stopping under a flood of traffic or connections beyond what the ESP32-S3's Wi-Fi and memory can reasonably handle

If you are unsure, please report it privately and I will triage it.

## Reporting a vulnerability

Please report vulnerabilities privately first:

* [report a vulnerability on GitHub](https://github.com/kf106/esp32-s3-minetest/security/advisories/new), or
* email [keir@chainfrog.com](mailto:keir@chainfrog.com)

A useful report includes the version or commit, the board (or the PC build), the steps or a script to reproduce the problem, and what happens. Reports that cannot be reproduced may be closed without further action.

I aim to acknowledge reports within two weeks. Depending on severity, I will either open a draft security advisory to work on the fix privately or ask you to file a public issue. The approach follows [responsible disclosure](https://en.wikipedia.org/wiki/Responsible_disclosure).

This is a one-person project maintained in my spare time, so please allow for that in response times. I am happy to credit reporters in the advisory (unless you prefer not to be named), but I cannot offer a bounty.
