flowhkp
=======

An incomplete, buggy, broken proxy server that translates just enough [OpenPGP
HTTP Keyserver Protocol (HKP)][hkp] into [Flowcrypt]'s [Attester] API to get
Thunderbird to resolve keys.

## Installation

Install it (`cargo install`), and point a systemd user service at it.

## Configuration

### Thunderbird

1. `Menu` -> `Settings` -> `Config Editor...`
1. Find the key `mail.openpgp.keyserver_list`
1. Append `, hkp://localhost` to the list of keyservers already present

[hkp]: https://datatracker.ietf.org/doc/html/draft-gallagher-openpgp-hkp
[Flowcrypt]: https://flowcrypt.com/
[Attester]: https://flowcrypt.com/attester/
