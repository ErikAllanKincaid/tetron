# Rayfish throughput / latency benchmark

Spins up **2 Scaleway instances in the same zone** and measures, for both
directions, the cost rayfish adds on top of the raw link:

- **latency** — `ping` mean RTT, direct (public IP) vs rayfish (`100.64.x.x` TUN)
- **throughput** — `iperf3` TCP, direct vs rayfish, forward (`tx`) and reverse (`rx`)

The two peers join an **open** network (`ray create --open` / `ray join <room>`),
so no invite handshake is needed.

## Prerequisites

Same as `tests/e2e`: an authenticated `scw` CLI, your SSH key registered in the
Scaleway account, Docker running (for `cross`), plus `jq` and `just`.

## Usage

```bash
tests/bench/provision.sh      # create 2 DEV1-S instances -> tests/bench/.servers
tests/bench/run.sh            # deploy ray + iperf3, peer, benchmark, print table
tests/bench/teardown.sh       # destroy the instances when done
```

Overrides: `ZONE`, `TYPE`, `IMAGE` (provision); `DURATION` (seconds/iperf run),
`SSH_KEY`, `KEEP_STATE=1` (skip the state wipe and re-use an existing network) for
`run.sh`. Results are printed and saved to `results/<stamp>.md` (+ `.raw` TSV).

## Caveats

`DEV1-S` has a single shared/burstable vCPU, so single-stream TCP is **CPU-bound**
— rayfish's userspace TUN + iroh QUIC datagram encryption is the bottleneck, and
absolute numbers are noisy run-to-run. Use a larger `TYPE` (e.g. `DEV1-L`,
`PRO2-S`) for steadier throughput; the *direct-vs-rayfish ratio* is the signal,
not the absolute Mbit/s. rayfish also runs an MTU of 1280 (the IPv6 minimum, per
WireGuard/Tailscale), which caps per-packet payload below the link's native MTU.
