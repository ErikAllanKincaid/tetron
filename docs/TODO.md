# tetron TODO

## High priority

- **Single-use invite keys as primary admission mechanism**: current model uses room id + live approval (`tetron requests`/`accept`). Intended model: coordinator mints single-use invite keys, shares them out-of-band, joiner auto-admitted on presentation (no approval queue). Room id becomes discovery-only. This reverses the MINIMAL-013 direction — invite minting should come back, but purpose-built for tetron's model (not full-tetron compat). Reusable keys for unattended fleets also needed.
