# Tetron commands.


## Install
## Download the release binary for x86_64 Linux
curl -Lo tetron https://github.com/ErikAllanKincaid/tetron/releases/download/nightly/tetron-linux-x86_64
## or
wget -O tetron https://github.com/ErikAllanKincaid/tetron/releases/download/nightly/tetron-linux-x86_64
## 
chmod +x tetron
sudo install tetron /usr/local/bin/tetron





## Invite
## Explicit duration:
tetron invite testnetwork create --expires 24h






## Leave
## voluntary departure.
tetron leave <net>
## coordinator removes a member.
tetron kick <net> <peer>
## destroy the entire network.
tetron nuke <net> [--force]


## Uninstall
## stop the daemon
sudo systemctl stop tetron
## tear down the test network first
sudo tetron nuke testnet
## disable auto-start
sudo systemctl disable tetron
## wipe config + identity (backup if needed)
sudo rm -rf /etc/tetron/
sudo rm /etc/systemd/system/tetron.service
sudo systemctl daemon-reload


























