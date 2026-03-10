flowhkp
=======

An incomplete, buggy, broken proxy server that translates just enough [OpenPGP
HTTP Keyserver Protocol (HKP)][hkp] into [Flowcrypt]'s [Attester] API to get
Thunderbird to resolve keys.

## Installation

1. `cargo build --release`
1. `sudo cp target/release/flowhkp /usr/local/bin/`
1. `sudo chown root:root /usr/local/bin/flowhkp`
1. `sudo cp systemd/flowhkp.service /etc/systemd/system/`
1. `sudo chown root:root /etc/systemd/system/flowhkp.service`
1. `sudo systemctl enable --now flowhkp.service`

## Configuration

### Thunderbird

1. `Menu` -> `Settings` -> `Config Editor...`
1. Find the key `mail.openpgp.keyserver_list`
1. Append `, hkp://localhost` to the list of keyservers already present

[hkp]: https://datatracker.ietf.org/doc/html/draft-gallagher-openpgp-hkp
[Flowcrypt]: https://flowcrypt.com/
[Attester]: https://flowcrypt.com/attester/
