# Pitopi Roadmap

## Done

- [x] Point-to-point tunnel between two peers
- [x] Multi-peer full mesh (N peers in one network)
- [x] Multiple simultaneous networks with isolation
- [x] Persistent network config
- [x] Three-word names for easy sharing
- [x] DHT membership publishing for offline coordinator resilience
- [x] Distributed ACLs with tag-based allow rules
- [x] Systemd/launchd service integration
- [x] Daemon architecture with Unix socket IPC

## Up Next

- [ ] Deterministic network simulator (TigerBeetle-style VOPR)
  - Abstract transport and TUN behind traits for injectable simulated network fabric
  - Seed-based deterministic replay of failure sequences
  - Scenarios: network partitions, ACL propagation under churn, reconnect behavior, split-brain, membership convergence after partition heal, race conditions between ACL updates and peer joins
- [ ] Social discovery (Discord, Slack, Steam)
- [ ] macOS Network Extension (no sudo)
- [ ] Windows, iOS, Android
