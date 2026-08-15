from libspec import Requirement, Constraint, UserStory

# --------------------------------------------------------------------------
# Requirements: subnet configurability (SUBNET-*)
# --------------------------------------------------------------------------

class SubnetField(Requirement):
    """REQUIREMENT-ID: SUBNET-001

    GroupBlob (src/membership.rs) gains `subnet: Option<(Ipv4Addr, u8)>`,
    following the existing `name: Option<String>` field's serde pattern
    (#[serde(default, skip_serializing_if = "Option::is_none")]). This is the
    network-wide signed source of truth every peer derives addresses against.
    """
    req_id = "SUBNET-001"


class SubnetCliFlag(Requirement):
    """REQUIREMENT-ID: SUBNET-002

    `torpedo create` gains `--subnet <CIDR>` (parsed to Ipv4Addr + prefix len).
    Omitting it falls back to the built-in default subnet (see SUBNET-011). The
    no-flag path keeps working; only the default value changes.
    """
    req_id = "SUBNET-002"


class DeriveIpParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-003

    derive_ip_with_index() (src/membership.rs) takes the network's subnet as
    a parameter instead of the hardcoded 0x6440_0000 base and fixed 22-bit host
    mask. Host-bit width is computed as 32 - prefix_len at call time. The mask
    computation, the netmask (SUBNET-005), and the gateway must all agree on
    the same prefix length or peers derive inconsistent addresses.
    """
    req_id = "SUBNET-003"


class RangeValidationParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-004

    ensure_in_cgnat_range() (src/membership.rs) validates a candidate IP
    against the network's own configured subnet (read from GroupBlob), not a
    single hardcoded 100.64.0.0/10 constant.
    """
    req_id = "SUBNET-004"


class TunCreateParameterized(Requirement):
    """REQUIREMENT-ID: SUBNET-005

    tun::create() (src/tun.rs) computes its netmask from the configured
    prefix length and its gateway as (base + 1), instead of the hardcoded
    (255, 192, 0, 0) netmask and 100.64.0.1 gateway.
    """
    req_id = "SUBNET-005"


class ConflictCheckRemoved(Requirement):
    """REQUIREMENT-ID: SUBNET-006

    check_cgnat_conflict() (src/tun.rs) and its call site are removed. This
    fork deliberately uses a subnet outside 100.64.0.0/10, so there is nothing
    for this check to protect against, and it is what currently blocks startup
    next to Tailscale.
    """
    req_id = "SUBNET-006"


# --------------------------------------------------------------------------
# Follow-up round: node subnet at boot (SUBNET-009/010).
# (UPGRADE-001 / CON-006 — the self-update requirement and its kill-switch
# constraint — were RETIRED by MINIMAL-002: tetron deletes the machinery
# outright, so absence replaces the gate.)
# --------------------------------------------------------------------------

class ConfigSetSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-009

    `torpedo config set subnet <CIDR>` (plus `config get subnet` / `config unset
    subnet`) persists the node's operative overlay subnet in AppConfig.subnet,
    mirroring the existing relay / discovery-dns / dns-upstreams config keys. The
    value is validated as a CIDR (via membership::parse_cidr) before persisting;
    `unset` (or empty) restores the built-in default subnet (SUBNET-011). Like
    the other config keys it takes effect at the next daemon restart (`sudo
    torpedo restart`),
    when the daemon builds its single TUN device and identity in that subnet.
    This removes the need to hand-edit settings.toml or rely on a create-time
    value to move the node's TUN off 100.64.0.0/10.
    """
    req_id = "SUBNET-009"


class CreateUsesNodeSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-010

    `torpedo create` with no `--subnet` uses the persisted node subnet
    (AppConfig.subnet) as the new network's GroupBlob.subnet, so the node's TUN
    and the network agree without specifying the subnet twice. `create --subnet
    <CIDR>` still works and also persists the node subnet, keeping a single
    source of truth for the node's one TUN. On a node with no persisted subnet
    yet, `create --subnet` sets it. If `--subnet` disagrees with an
    already-persisted node subnet it is rejected with a clear error ("node
    subnet is <Y>; change it with `torpedo config set subnet` + restart first"),
    never silently producing a network the node's single TUN cannot carry.
    """
    req_id = "SUBNET-010"


class DefaultSubnetSafe(Requirement):
    """REQUIREMENT-ID: SUBNET-011

    The built-in default overlay subnet (membership::default_subnet, used when a
    GroupBlob's / config's subnet is None) changes from 100.64.0.0/10 to
    10.88.0.0/24 — an uncommon 10.x slice deliberately chosen NOT to collide
    with Tailscale's 100.64.0.0/10, so a no-flag `tetron create` coexists with
    Tailscale out of the box. `--subnet` / `config set subnet` still override it.
    A /24 gives 256 host addresses, enough for personal/team meshes; users who
    need more can set a larger prefix explicitly.
    """
    req_id = "SUBNET-011"


class SubnetOverlapGuard(Requirement):
    """REQUIREMENT-ID: SUBNET-012

    At daemon startup the node rejects (refuses to start the data plane) if its
    configured overlay subnet overlaps an existing local interface / route, with
    a clear error telling the user to pick another via `torpedo config set
    subnet`. This is a NEW, subnet-aware guard — NOT a revival of the removed
    hardcoded check_cgnat_conflict (SUBNET-006): that one refused whenever any
    100.64.0.0/10 address was present (i.e. whenever Tailscale ran); this one
    only refuses on a genuine overlap between the *chosen* overlay subnet and a
    real local network, so it protects the host's routing without blocking the
    Tailscale-coexistence case (10.88.0.0/16 vs Tailscale's 100.64.0.0/10 do not
    overlap). Pairs with SUBNET-011: the safe default plus this guard mean a
    bad range fails loudly instead of hijacking the host's routes.
    """
    req_id = "SUBNET-012"


class DefaultSubnetDocsAccurate(Requirement):
    """REQUIREMENT-ID: SUBNET-013

    User-facing help text and doc-strings state the ACTUAL default overlay
    subnet (10.88.0.0/24), not the old 100.64.0.0/10 that SUBNET-011 replaced:
    - `tetron create --subnet` CLI help (src/main.rs) says the default is
      10.88.0.0/24.
    - The GroupBlob.subnet (src/membership.rs) and AppConfig.subnet
      (src/config.rs) field docs, and the IPC Create.subnet doc
      (tetron-proto/src/ipc.rs), describe `None` as the 10.88.0.0/24 default.

    Explicitly OUT OF SCOPE (documented deferrals, not the fork's Linux path,
    decision left for later): the macOS `route_peer_range` branch (src/tun.rs),
    the Android VpnService (android/), and the upstream e2e/bench shell harnesses
    (tests/) still assume 100.64.0.0/10. They are adapted or removed in a future
    project, not here.
    """
    req_id = "SUBNET-013"


class SubnetChangeObservableAndAnnounced(Requirement):
    """REQUIREMENT-ID: SUBNET-014

    Two subnet-UX fixes found in Phase-7 live testing.

    (1) `create --subnet X` / `join` onto a network whose subnet differs from this
    node's live TUN persist the subnet but only apply it to the TUN at the next
    (re)start. Previously silent, so the node kept its old subnet while the roster
    advertised the new one and NO IP forwarding worked until a manual restart. The
    `Created`/`Joined` IPC responses now carry an optional `warning`; the CLI
    prints it when the chosen subnet != the live TUN subnet ("subnet B/P takes
    effect after `sudo torpedo restart`"). The pure helper is
    `membership::subnet_change_warning`.

    (2) `config get` as a non-root user cannot read the 0600 root:root
    settings.toml (it holds contact_secret_key, so its perms must NOT be relaxed),
    so config::load() silently returned defaults and misreported e.g. `subnet` as
    <default> while the node ran on 10.99. `config get` now detects the unreadable
    file and errors with a "re-run with sudo" hint instead of a wrong value;
    `sudo torpedo config get` shows the real value. Full read-via-daemon IPC is a
    deferred follow-up.

    ENFORCEMENT: unit test on subnet_change_warning (reconcile's `test` check).
    """
    req_id = "SUBNET-014"


class TestHarnessSubnetUpdated(Requirement):
    """REQUIREMENT-ID: SUBNET-015

    Found while doing RENAME-017 (2026-07-10): the `tests/` harness still assumed
    upstream's `100.64.0.0/10` CGNAT range and the pre-fork fixed magic-DNS IP
    `100.100.100.53`, both changed by the fork's core purpose — the default
    overlay is now `10.88.0.0/16` (SUBNET-011) and the resolver is subnet-derived
    to `10.88.100.53` (SUBNET-007/008). Two FUNCTIONAL breaks, not doc drift:

    - `tests/lib/common.sh` `own_ip()` grepped status output for
      `100\\.[0-9]+\\.[0-9]+\\.[0-9]+` — matches nothing in a real `10.88.x.x`
      address, so it returned an empty string and the five tests that derive a
      node's VPN IP from it (device-cert, ssh, unpair, bench, connect) fed empty
      IPs into pings/asserts. Regex → `10\\.88\\.[0-9]+\\.[0-9]+`.
    - `tests/e2e/dns/run.sh` set `MAGIC=100.100.100.53` and queried it; the
      resolver answers at `10.88.100.53`. → `MAGIC=10.88.100.53`.

    Plus 6 comment/README references to `100.64.x.x` / `100.64.0.0/10` /
    `100.100.100.53` reworded to the `10.88` reality. No test sets a custom
    `--subnet`, so the exact `10.88` literals are correct for the whole suite.

    ENFORCEMENT: CON-012 (below). Distinct from CON-002 (`grep_hardcoded_cgnat`),
    which polices the same drift in `src/` (membership/tun/dns).
    """
    req_id = "SUBNET-015"


# --------------------------------------------------------------------------
# SUBNET-BUG-001: TUN created with local subnet, not network's subnet
# --------------------------------------------------------------------------

class SubnetMismatchOnJoin(Requirement):
    """REQUIREMENT-ID: SUBNET-BUG-001

    When a node joins a network whose overlay subnet differs from the node's
    locally configured subnet (from `tetron config set subnet` or the
    default), the TUN device is created with the *local* subnet, not the
    network's authoritative subnet from the `GroupBlob`. The member is
    assigned a mesh IP from the network's subnet (visible in `tetron status`
    and the signed roster), but the TUN interface carries an IP from the
    local subnet. Packets addressed to the member's correct mesh IP arrive
    via QUIC but are written to a TUN whose IP is in a different range --
    the kernel does not recognise the dst IP as local and drops the packet.
    This silently breaks the data plane (no ping, no TCP) with no error
    logged anywhere.

    Fix: reject the join in `join_network_inner` with a clear error
    message when the network's subnet (from `GroupBlob.subnet`) differs
    from the node's local subnet (`config::node_subnet()`). The error
    tells the user to run `sudo tetron config set subnet <network-cidr>
    && sudo tetron restart` and try again before joining. This matches
    the pattern already used by `tetron create --subnet` which rejects a
    `--subnet` that disagrees with the persisted node subnet (lines
    260-264 in create_join.rs).

    The existing persist-on-join code in `finalize_join` (lines 1199-1204
    in create_join.rs) that calls `config::set_node_subnet(joined_subnet)`
    is retained: when subnets already match, it redundantly persists the
    value, ensuring the next restart rebuilds the TUN in the correct
    subnet even if config was somehow reset.

    (Per-network TUN devices or policy routing — option (c) — is the
    correct long-term fix and is documented in SUBNET_COLLISION.md as
    deferred.)

    Found: 2026-07-15, network "shallows" with AORUS (10.77.0.0/24) and
    usbos-1 (10.88.0.0/16). Tested: 2026-07-16, network "test-tetronnet"
    with 590i-aorus-ultra, xps-17-9720, X10SRA, xeon40 (10.55.55.0/24).
    """
    req_id = "SUBNET-BUG-001"


# --------------------------------------------------------------------------
# FRAG-001: IPv4 fragmentation for QUIC datagram size limits
# --------------------------------------------------------------------------

class Ipv4Fragmentation(Requirement):
    """REQUIREMENT-ID: FRAG-001

    When the QUIC connection's `max_datagram_size()` is smaller than the TUN
    MTU (1280), IP packets read from the TUN device will not fit in a single
    QUIC datagram. The forwarder must fragment oversize IPv4 packets into RFC
    791-compliant IP fragments, each sent as a separate QUIC datagram, so TCP
    connections (SSH, HTTP, etc.) do not stall.

    Fragment payload size is rounded down to the nearest multiple of 8 bytes
    (RFC 791 Section 3.2). Each fragment carries the original IP header with
    updated Total Length, More-Fragments flag, Fragment Offset, and a
    recalculated header checksum. The original identification and Don't
    Fragment flag are preserved.

    Receiving kernel reassembles fragments before delivery -- no reassembly
    logic is needed in the daemon.

    IPv6 fragmentation is not yet implemented and oversized IPv6 packets are
    dropped with a warning log entry.

    Found: 2026-07-15, network "shallows" where Quinn's max_datagram_size
    was 1162-1192, below the 1228-byte TCP segments produced at TUN MTU 1280.
    SSH key exchange stalled silently at "expecting SSH2_MSG_KEX_ECDH_REPLY".

    Follow-up (F-04, security-audit finding, 2026-07-23): `fragment_ipv4`
    read the original packet's header fields (identification, DF flag, IHL)
    and forwarded fragments built from them without ever checking that the
    original header's own checksum was valid -- each fragment got a freshly
    computed, valid checksum regardless, so a corrupted or malformed header
    was silently "healed" rather than rejected the way normal IP processing
    would. Fixed: `fragment_ipv4` now verifies the stored header checksum
    against a fresh computation before trusting any field or fragmenting at
    all, returning `None` (already the existing "cannot fragment, malformed"
    path in `forward.rs`, unchanged) on a mismatch.

    Follow-up (live-testing regression, found 2026-07-24 on a real 3-machine
    mesh -- coordinator + 2 members, non-default subnet, fresh HARDEN-002/
    004/005 + FRAG-002 build): F-04's checksum-verify line read
    `!ip_checksum(&hdr) != stored_csum` -- a stray extra bitwise NOT that
    made the comparison *always* fail, since `ip_checksum` already applies
    the final one's-complement internally (confirmed against a known-good
    textbook header checksum: `ip_checksum(hdr)` alone matches, `!ip_checksum
    (hdr)` never can, for any input -- X and ~X are never equal). Every real
    oversized IPv4 packet was therefore treated as "corrupt" and dropped
    outright -- a full regression of the original FRAG-001/F-04 bug, hit live
    at the *exact* MTU numbers from the original 2026-07-15 report
    (`max=1162`, `len=1228`), confirmed via `tetron`'s own log line
    (`cannot fragment IPv4 packet (options or malformed)`). Masked in a
    quick smoke test only because TCP's own retransmission/backoff still got
    the data through (just much slower); a full 20MB scp over the affected
    IPv4 mesh address completed with a correct end-to-end checksum despite
    the drops, which is why this needed a real bulk transfer under log
    inspection to surface, not just a ping/login check. The *same* stray `!`
    was also present on the fragment-*write* side (present since FRAG-001's
    original commit, unrelated to F-04) and in every test helper that
    constructs a synthetic packet checksum -- so the entire existing test
    suite was self-consistently checking its own (equally inverted)
    convention rather than RFC 1071 truth, which is why no unit test ever
    caught it; the only prior checksum test exercised "corrupt gets
    rejected," never "valid gets accepted." Fixed by removing the extra `!`
    at both the verify and write sites (and in the test helpers), and added
    `frag_accepts_a_genuinely_valid_header` (closes the missing "valid gets
    accepted" direction) plus `ip_checksum_matches_a_known_good_real_world_
    header` (an independent ground-truth check against a textbook checksum,
    immune to the whole class of "both sides agree with each other, neither
    is right" bug). Re-verified live after the fix: the same 3-machine mesh,
    same MTU condition, now logs `fragmenting oversized IP packet ...
    nfrags=2` instead of the drop warning, and the 20MB transfer's checksum
    still matches with zero warnings logged.

    Follow-up (`FRAG-004`, 2026-08-15): the "each fragment carries the
    original IP header with updated Total Length, More-Fragments flag,
    Fragment Offset" wording above is correct only for an *unfragmented*
    input packet. `FRAG-004` amends it for the case where the packet read
    off the TUN is already an IP fragment itself, which this requirement
    never considered. `FRAG-005` and `FRAG-006` further amend the same
    function's input validation and allocation behavior respectively.
    Fragmenting a packet whose DF flag is set stays deliberate and
    unchanged: tetron is the encapsulating tunnel here, not a router on
    the path, and the host stack's own path MTU (the TUN's 1280) is not
    what forced the split -- the peer connection's `max_datagram_size`
    is, which the host cannot see or act on. The fragments therefore
    carry DF set, which no receiving stack tetron targets consults during
    reassembly.
    """
    req_id = "FRAG-001"


# --------------------------------------------------------------------------
# FRAG-002: IPv6 fragmentation for QUIC datagram size limits
# --------------------------------------------------------------------------

class Ipv6Fragmentation(Requirement):
    """REQUIREMENT-ID: FRAG-002

    Closes the gap `FRAG-001` explicitly left open: an oversized IPv6 packet
    hitting `route.conn.max_datagram_size()` in `src/forward.rs` was dropped
    outright ("IPv6 fragmentation is not yet implemented"), so any mesh
    traffic between two peers' tetron IPv6 addresses (the `200::/7` range)
    was exposed to the same silent-stall bug FRAG-001 fixed for IPv4 -- e.g.
    an SSH session over a peer's IPv6 mesh address hanging at KEX exactly
    like the original FRAG-001 report, unfixed by FRAG-001 alone.

    Unlike IPv4, an IPv6 packet has no fragmentation fields in its base
    header -- RFC 8200 SS4.5 requires a separate Fragment extension header,
    and by RFC 8200's own rule only the packet's true originating host may
    ever fragment an IPv6 packet; there is no in-network router
    fragmentation the way IPv4 allows. A literal RFC 8200 implementation
    would also depend on the receiving peer's *kernel* recognizing and
    reassembling that extension header the way it does for real IPv6
    fragments delivered over a physical link.

    tetron is not a generic router relaying between two unrelated stacks
    here: it reads the whole packet off its own TUN (the true origin) and
    the reassembling party is its own code on the receiving peer (not a
    generic IP stack) -- `forward::spawn_peer_reader`. So FRAG-002 is
    implemented as a **tetron-internal protocol concern**, not a literal RFC
    8200 extension header:

    - `packet::fragment_ipv6(packet, id, max_size)` splits an oversized IPv6
      packet into wire envelopes, each `<= max_size` bytes: a 1-byte magic
      (`FRAG6_MAGIC = 0xF6`, chosen so its top nibble -- 0xF -- can never
      collide with a real IP packet's version nibble, 4 or 6), a 4-byte
      fragment-set id, a 2-byte byte offset, a 1-byte more-fragments flag,
      then the payload slice. `id` disambiguates concurrent/overlapping
      fragment sets on the same connection from each other; the sender uses
      a per-process monotonic counter.
    - `packet::Ipv6Reassembler` is per-peer-connection state living inside
      `spawn_peer_reader`'s own task (no locking needed -- one reader per
      peer connection). `accept(datagram)` returns `Complete(Vec<u8>)` once
      every fragment for an id has arrived (checked via a `BTreeMap` keyed
      by offset -- complete only when contiguous from 0 to the total length
      the final fragment revealed), `Waiting` while incomplete, `Rejected`
      for a malformed/truncated envelope or one whose claimed offset would
      grow the reassembled packet past `tun::TUN_MTU` (1280) -- no packet
      `fragment_ipv6` ever produced can legitimately claim to be bigger than
      the TUN device's own MTU, so a bigger claim is either a bug or a
      malicious peer, never legitimate mesh traffic -- and `NotAFragment`
      for anything not carrying the magic byte (the ordinary, unfragmented
      path, unchanged).
    - Bounded against a peer that opens many fragment sets and never
      completes any of them (accidental or malicious): at most 16 concurrent
      in-flight ids per connection (oldest evicted past that), and any id
      untouched for 5 seconds is garbage-collected. Both bounds are cheap
      because a real fragmented packet here is bounded at `TUN_MTU` (1280
      bytes) and in practice almost always splits into exactly two
      fragments.
    - `forward::run_mesh`'s `Some(6) => { ... }` arm mirrors the `Some(4)`
      arm: calls `fragment_ipv6` with a fresh id from `NEXT_FRAG6_ID`, sends
      each envelope as its own QUIC datagram, drops with a warning log only
      when `max_dgram` is too small to fit even the 8-byte envelope header
      plus one payload byte (practically never, since `max_dgram` is always
      well above that in real QUIC connections).

    A reassembled/rejected fragment is accounted in `stats::DropReason` the
    same as any other inbound packet (`Malformed` on `Rejected`); a `Waiting`
    outcome records nothing yet, matching the existing behavior that stats
    are recorded once a deliverable unit exists, not per raw wire datagram.
    """
    req_id = "FRAG-002"


# --------------------------------------------------------------------------
# FRAG-003: backpressure check missing from the fragmentation send paths
# --------------------------------------------------------------------------

class FragmentationBackpressure(Requirement):
    """REQUIREMENT-ID: FRAG-003

    Found 2026-08-14 during the OOM-leak investigation's cross-machine
    `tetron status --json` comparison: xps-17-9720 fragments IPv4 packets
    at a per-packet rate ~13.7x aorus's, and is also the machine showing
    the fastest, most consistently accelerating RSS growth in that
    investigation. Reading `forward.rs`'s send paths directly (not
    assumed) turned up a real asymmetry present unchanged since
    `FRAG-001`'s very first commit, one month prior: the standard
    (non-fragmented) send path checks
    `route.conn.datagram_send_buffer_space() < n` before calling
    `send_datagram`, deliberately dropping the *new* packet (drop-newest)
    rather than letting iroh's own queue evict an older one -- but
    neither the `FRAG-001` IPv4 fragmentation branch nor the `FRAG-002`
    IPv6 fragmentation branch has any equivalent check. Under exactly the
    congestion conditions that make fragmentation more likely in the
    first place (a shrunk `max_datagram_size`), fragments bypass the
    deliberate backpressure policy entirely and fall straight through to
    `send_datagram`.

    Traffic-volume correlation alone was investigated and explicitly
    ruled insufficient as an explanation on its own (USER, 2026-08-14:
    "Traffic should not lead to memory-leak anyway") -- a well-behaved
    forwarding path should not leak regardless of how much legitimate
    traffic flows through it. This requirement is not a claim that the
    missing check *is* the dominant driver of that investigation's
    observed growth; it is a real, independently-found correctness gap
    in tetron's own code, in exactly the send path a real machine
    exercises disproportionately, worth closing on its own merits.

    **Fix**: both fragmentation branches now check
    `datagram_send_buffer_space()` before sending each fragment, exactly
    as the standard path already does. Unlike the standard path (one
    packet, one check), a fragment set is only useful to the receiver if
    *every* fragment arrives -- IPv4's kernel reassembly and `FRAG-002`'s
    own `Ipv6Reassembler` both discard an incomplete set once its GC
    timeout elapses, so a fragment sent into a set that will never
    complete is wasted bandwidth and buffer space, not partial progress.
    So the check runs per-fragment inside the send loop: the first
    fragment that doesn't fit stops the loop entirely (`break`, not
    `continue`) rather than skipping just that one fragment and
    attempting the rest -- and records exactly one `Backpressure` drop
    for the whole abandoned packet, not one per already-unsent fragment,
    matching how the standard path records one drop per packet rather
    than per byte.

    Neither fragmentation *behavior* (which packets get fragmented, how)
    nor reconnect/multipath logic changes -- this is a send-side
    admission check only, identical in kind to what `FRAG-001`'s own
    standard path has always had.
    """

    req_id = "FRAG-003"


# --------------------------------------------------------------------------
# FRAG-004: re-fragmenting an already-fragmented IPv4 packet
# --------------------------------------------------------------------------

class Ipv4RefragmentationPreservesOffset(Requirement):
    """REQUIREMENT-ID: FRAG-004

    Found 2026-08-15 by reading `packet::fragment_ipv4` directly during an
    MTU/fragmentation audit, then confirmed against the real function with
    a throwaway probe. `FRAG-001` assumed the packet it reads off the TUN
    is a whole, unfragmented IP datagram. That assumption is wrong: the
    TUN's MTU is 1280 (`tun::TUN_MTU`), so the host kernel itself
    fragments anything larger before tetron ever sees it, and each of
    those fragments arrives at `forward::run_mesh` as an ordinary IPv4
    packet carrying a non-zero Fragment Offset, a set More-Fragments flag,
    or both. Whenever the peer connection's `max_datagram_size` is below
    1280 -- the ordinary case on a relay path, and on *every* path for the
    first few round trips, since noq's default `initial_mtu` is 1200 --
    such a fragment is oversized and gets re-fragmented.

    `fragment_ipv4` overwrote both fragmentation fields unconditionally:
    the Fragment Offset was recomputed from its own loop position starting
    at zero, and More-Fragments was set from that loop position alone
    (cleared on the last sub-fragment). Only the DF flag was carried over
    from the input header. So re-fragmenting a middle fragment produced
    sub-fragments claiming byte positions that belong to the *first*
    fragment of the datagram, and a cleared More-Fragments flag declaring
    an end to a datagram that had not ended. Measured on the real function
    (input: a middle fragment at byte offset 1480 with MF=1, `max_size`
    1200) the two sub-fragments came out at byte offsets 0 and 1176 with
    MF true then false, where 1480 and 2656 with MF true on both is
    correct.

    Consequence, and why it stayed hidden: the receiving kernel either
    drops the whole datagram (two fragments both claiming to be last) or
    reassembles corrupt bytes, and tetron records no drop of any kind for
    it -- so nothing surfaces in `tetron status`, in the `MTU-DIAG-001`
    drop breakdown, or in the logs. `fragmented_ipv4` counts up exactly as
    it does for a correct split. The traffic that triggers it is UDP or
    ICMP larger than the TUN MTU (`ping -s 2000`, a large DNS answer over
    UDP, an `iperf3 -u` run at its 1460-byte default payload); bulk TCP
    never triggers it, because TCP sizes its own segments to the 1280 MTU
    and so never hands the kernel anything to pre-fragment. That is why
    `FRAG-001`'s and `FRAG-002`'s live verification (a 20MB scp, and SSH
    sessions) could pass end-to-end with correct checksums while this was
    broken the whole time.

    **Fix**: `fragment_ipv4` reads the input header's Fragment Offset and
    More-Fragments flag, and:

    - adds the input's byte offset to each sub-fragment's own offset, so
      sub-fragment offsets are absolute within the original datagram
      rather than relative to the fragment being split;
    - ORs the input's More-Fragments flag into the last sub-fragment's,
      so a split middle fragment keeps MF set and only a split *last*
      fragment clears it;
    - refuses the packet outright (`None`, the existing "cannot fragment"
      path) when an absolute offset would not fit the header's 13-bit
      Fragment Offset field, i.e. when it would exceed 65528 bytes. Only
      reachable from an input header that is already illegal, since no
      IPv4 datagram may exceed 65535 bytes total, but the addition is
      this requirement's own arithmetic and silently truncating it would
      reintroduce exactly the class of corruption being fixed.

    Both are no-ops for the unfragmented input `FRAG-001` described (input
    offset 0, input MF clear), so that requirement's stated behavior is
    preserved exactly for the case it actually covered.

    Note the asymmetry with `FRAG-002`: the IPv6 path was never exposed to
    this, because its tetron-internal envelope wraps the packet opaquely
    and never rewrites IP header fields. A kernel-produced IPv6 fragment
    (Fragment extension header, next-header 44) passes through the
    envelope untouched and is reassembled by the receiving kernel from its
    own unmodified extension header.

    Depends on nothing; `FRAG-005` and `FRAG-006` touch the same function
    but neither depends on this one, so the three may land in any order.
    Ordered first here only so it can be cherry-picked alone if a release
    is needed before the other two are ready.
    """

    req_id = "FRAG-004"


# --------------------------------------------------------------------------
# FRAG-005: bounds guards on peer-influenced fragmentation inputs
# --------------------------------------------------------------------------

class FragmentationInputBounds(Requirement):
    """REQUIREMENT-ID: FRAG-005

    Found 2026-08-15 in the same audit as `FRAG-004`. `fragment_ipv4`
    computes `let max_payload = (max_size - HEADER_LEN) & !7;` with no
    prior check that `max_size` is at least `HEADER_LEN` (20). `max_size`
    is `Connection::max_datagram_size()`, which noq computes as the
    minimum of the current path MTU budget and *the remote peer's
    advertised* `max_datagram_frame_size` transport parameter
    (`vendor/noq-proto-1.1.0/src/connection/datagrams.rs`, `max_size`):
    the value is therefore partly under a remote peer's control, and a
    peer advertising a small enough frame size drives it below 20.

    Confirmed by calling the real function with `max_size = 10`: it
    panics with "attempt to subtract with overflow". Release builds set no
    `overflow-checks` (`Cargo.toml`'s `[profile.release]`), so a release
    daemon wraps instead, producing a single oversized "fragment" that
    `send_datagram` then rejects -- wrong, but contained. A debug or test
    build panics, and `main::install_panic_hook` turns a daemon panic into
    `process::abort()`, so an admitted peer could halt another node's
    daemon by advertising a hostile transport parameter. Admission still
    gates who can do this (an invite key is required to reach the data
    path at all), which is why this is hardening rather than a
    remote-unauthenticated hole.

    **Fix**, both in `packet.rs`:

    - `fragment_ipv4` rejects `max_size < HEADER_LEN + 8` up front,
      returning `None` (the existing, unchanged "cannot fragment" path in
      `forward.rs`, which records `DropReason::FragmentationFailed`).
      `+ 8` rather than `+ 1` because RFC 791 requires every non-final
      fragment's payload to be a multiple of 8 bytes, so a `max_size`
      leaving fewer than 8 payload bytes cannot produce a legal fragment
      set anyway -- exactly what the existing `max_payload < 8` check
      already concluded, now reached without underflowing first.
    - `fragment_ipv6` refuses a packet longer than `u16::MAX`, whose byte
      offset would silently truncate in the envelope's 2-byte offset
      field (`offset as u16`). Not reachable today, since the TUN never
      delivers more than `TUN_MTU` bytes, but the cast is unguarded and
      the envelope format is tetron's own to keep honest. `fragment_ipv6`
      already guards its `max_size` correctly
      (`max_size <= FRAG6_HEADER_LEN`), so no change is needed there.

    No behavior change for any real connection: a healthy peer's
    `max_datagram_size` is over a kilobyte, and both guards are
    unreachable in normal operation. Independent of `FRAG-004` and
    `FRAG-006`.
    """

    req_id = "FRAG-005"


# --------------------------------------------------------------------------
# IPV4-MIN-IHL-001: reject IPv4 headers shorter than five words (upstream
# 6d008d5 `fix(firewall)`, ported from the 2026-08-05 upstream review)
# --------------------------------------------------------------------------

class Ipv4MinimumIhl(Requirement):
    """REQUIREMENT-ID: IPV4-MIN-IHL-001

    `packet::parse_ipv4` (`src/packet.rs`) accepted `ihl < 5`: the length
    check `packet.len() < header_len` is always satisfied by a short IHL
    (e.g. `ihl = 1` gives `header_len = 4`, and a 20+ byte packet trivially
    passes), so "ports"/TCP-flags/ICMP fields were read from bytes inside
    the IP header. Every OS drops such headers on receive anyway (RFC 791:
    IHL must be at least 5).

    Fix (ported from upstream `6d008d5`): reject `ihl < 5` outright at the
    top of `parse_ipv4`, returning `None` like any other malformed header.

    Impact today is nil — tetron's consumers read only fixed-offset
    fields (`evaluate_inbound` uses `info.src_ip`; the TUN-routing path
    uses `info.dst_ip`; neither depends on IHL) — so this is defense-in-
    depth: the parser advertises "packet info," and the next consumer of
    `PacketInfo`'s ports/flags/icmp fields would otherwise inherit the bug.
    Modeled on the existing `parse_too_short` test; new test
    `parse_rejects_short_ihl` covers `ihl < 5` with a long-enough packet.

    Independent of INVITE-CHECKSUM-001, DHT-ERRCAUSE-001, and
    TUN-SENDERCACHE-001: disjoint files, no shared state. May land in any
    order.

    Found: 2026-08-05, upstream rayfish review `a56b4b9..b002168`
    (`DO-NOT-COMMIT/REVIEW_upstream-rayfish_2026-08-05.md`, item 4).
    """
    req_id = "IPV4-MIN-IHL-001"


# --------------------------------------------------------------------------
# TUN-SENDERCACHE-001: per-reader arc_swap Cache for the swappable TUN
# writer (upstream e537db6 `perf(forward)`, ported from the 2026-08-05
# upstream review)
# --------------------------------------------------------------------------

class TunSenderCache(Requirement):
    """REQUIREMENT-ID: TUN-SENDERCACHE-001

    Each per-peer reader (`forward::spawn_peer_reader`) resolves the
    swappable TUN writer with `tun_tx.load_full()` on every inbound
    datagram — two atomic refcount operations on the hottest inbound path.
    The writer is only ever swapped on a TUN re-attach (VPN toggle), so the
    per-packet resolution is almost always redundant.

    Fix (ported from upstream `e537db6`): give each reader an
    `arc_swap::cache::Cache` built once at spawn time
    (`Cache::new(tun_tx)`), and resolve via `cache.load()` per datagram.
    The cache revalidates against the cell and reuses the held value,
    cloning only when a re-attach actually stores a new sender. tetron
    already depends on `arc_swap` (1.9.2) and the cell is the same
    `Arc<arc_swap::ArcSwap<mpsc::Sender<Bytes>>>` shape upstream
    optimized; the Cache is a drop-in swap at the read site
    (`src/forward.rs`, `spawn_peer_reader`'s `Accept` arm).

    Zero behavior change: the sender still resolves per datagram with the
    same swap semantics across TUN attach/detach cycles (the Cache
    revalidates on every `load`, so a detach + re-attach is picked up on
    the next packet). Upstream measured 11.0 ns -> 1.0 ns per packet on
    its `writer_resolve` bench; the optional `benches/forward.rs`
    microbench is not required for this port.

    Independent of INVITE-CHECKSUM-001, DHT-ERRCAUSE-001, and
    IPV4-MIN-IHL-001: disjoint files, no shared state. May land in any
    order.

    Found: 2026-08-05, upstream rayfish review `a56b4b9..b002168`
    (`DO-NOT-COMMIT/REVIEW_upstream-rayfish_2026-08-05.md`, item 3).
    """
    req_id = "TUN-SENDERCACHE-001"


# --------------------------------------------------------------------------
# MULTISEG-001: per-network subnet field on NetworkConfig (additive, unread)
# --------------------------------------------------------------------------

class PerNetworkSubnetConfigField(Requirement):
    """REQUIREMENT-ID: MULTISEG-001

    Step 1 of the multi-segment TUN plan (scoped in full in
    `DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`, "Scoped code changes"):
    tetron today shares one TUN device and one node-wide overlay subnet
    (`AppConfig.subnet`, SUBNET-010) across every joined network, even
    though each network's signed `GroupBlob` already carries its own
    `subnet: Option<Subnet>` — the data model has supported per-network
    subnets since BLOB-001; only the daemon's single-TUN orchestration
    hasn't caught up. Multi-segment TUN (one TUN device + subnet per
    network, so a host can bridge two operator-distinct segments the way
    two physical NICs would) needs a place to persist each network's own
    subnet locally, ahead of any per-network TUN device existing to use it.

    Adds `subnet: Option<crate::membership::Subnet>` to `NetworkConfig`
    (`src/config.rs`), serialized the same way as the existing node-wide
    `AppConfig.subnet` / `Settings.subnet` fields (`with =
    "crate::membership::cidr_opt"`, CIDR string on disk, `None` omitted).
    `None` means "this network uses the node-wide subnet," identical to
    today's actual behavior — so this field starts fully inert. The three
    non-test `NetworkConfig` construction sites: `create_join.rs`'s
    `create_network_inner` and `join.rs`'s `join_network_inner` set it to
    `None` (nothing mints a per-network subnet yet); `runtime.rs`'s
    `restore_coordinator_network` carries the persisted value forward
    (`net_config.and_then(|nc| nc.subnet)`), matching the existing
    preserve-across-restart pattern already used for `admins`/`direct`.

    Deliberately scoped to *only* this field — no `--subnet` CLI wiring, no
    read site, no interaction with `SUBNET-010`'s node-wide-subnet
    enforcement (both its sites are untouched). This is intentionally the
    one part of the multi-segment TUN plan that is safe and independently
    shippable on its own: the field is round-tripped by serde but nothing
    in the daemon ever reads it, so there is no behavior change and no way
    for this commit alone to reintroduce SUBNET-BUG-001 (a previously-fixed
    bug where a subnet mismatch silently misconfigured the single shared
    TUN). Every later step in the plan (relaxing `SUBNET-010`'s join-side
    check, per-network `NetworkHandle`/`MeshCtx`/TUN-lifecycle
    restructuring, `forward.rs`) depends on this field existing first, and
    is unsafe to land before per-network TUN devices actually exist to
    honor it -- see the corrected "Suggested commit sequence" in the ideas
    doc.

    Found: 2026-07-18, first commit of the `feat/multi-segment-tun` branch.
    """
    req_id = "MULTISEG-001"


# --------------------------------------------------------------------------
# MULTISEG-002: per-network PeerTable/MeshCtx (NetworkHandle owns its own
# data-plane routing table instead of sharing one daemon-wide table)
# --------------------------------------------------------------------------

class PerNetworkPeerTableAndMeshCtx(Requirement):
    """REQUIREMENT-ID: MULTISEG-002

    Step 3 of the multi-segment TUN plan. Moves `PeerTable` off `MeshManager`
    (previously one daemon-wide table shared by every joined network) onto
    each `NetworkHandle` — every network now owns its own routing table,
    populated as soon as the handle exists (independent of whether a TUN is
    attached yet, matching the pre-existing headless-before-attach pattern
    `build_headless()` already relied on).

    `MeshCtx` (the per-accept-handler/background-task bundle of
    `identity`/`peers`/`tun_tx`/`stats`/`blob_store`/`pruned_peers`) is no
    longer built once daemon-wide via a `mesh_ctx()` method. Two construction
    paths now exist: (1) every call site that establishes a network — the
    `create_network_inner`/`join_network_inner`/`restore_coordinator_network`
    handlers, plus the `try_dht_fallback_join` dead-code path kept compiling
    for consistency — builds a fresh `MeshCtx` from a freshly created
    `peers`/placeholder `tun_tx` pair (`MeshManager::new_network_data_plane`)
    *before* the `NetworkHandle` exists in `self.networks`, since there is
    nothing yet to look up; (2) `MeshManager::mesh_ctx_for(network)` looks up
    an *existing* handle's own `peers`/`tun_tx`, used only by
    `promote_to_coordinator` (the one call site where the handle already
    exists). `register_coordinator_handler` and `spawn_coordinator_background_
    tasks` both take `ctx: MeshCtx`/`ctx: &MeshCtx` as an explicit parameter
    now, rather than building it internally, so each caller supplies whichever
    of the two is correct for its situation.

    **Deliberate deviation from the original scoping doc
    (`DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`):** the doc suggested
    `PeerEntry.conns: HashMap<SmolStr, Connection>` could collapse to a bare
    `Connection` once each network has its own table (a peer only ever has one
    connection within a single-network-scoped table). Implemented instead as
    **N separate instances of the existing `PeerTable`/`PeerEntry` shape,
    unchanged** — `src/peers.rs` has zero code changes. Reasoning: the
    `conns`-collapse would touch every one of `PeerTable`'s ~15 methods'
    signatures (dropping their `network: &str` parameter) and every call site
    across `accept.rs`/`join.rs`/`runtime.rs`/`create_join.rs`/
    `diagnostics.rs`/`admin.rs`/`publish.rs` — a second, independently risky
    refactor layered on top of an already-large one, for a data-structure
    tidiness gain with no behavioral difference (a table now holding only one
    network's entries makes the existing `_for_network`/`_by_network`-suffixed
    methods over-general but not incorrect — calling
    `peers_for_network_with_conn(name)` on a table that only ever contained
    `name`'s entries returns exactly the same thing a hypothetical
    `all_with_conn()` would). Chose the smaller, safer diff. Flagged here as a
    real follow-up cleanup, not silently dropped.

    `crate::peercache::refresh_from_peers` (CACHE-001) took one `&PeerTable`;
    with N tables it is now called once per network via a new
    `MeshManager::refresh_peer_cache()` that iterates `self.networks`.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-003/004/005/006 (see MULTISEG-004's "Suggested commit sequence"
    note in the ideas doc for why these five could not safely ship as
    separate commits despite being granular, separable requirements).
    """
    req_id = "MULTISEG-002"


# --------------------------------------------------------------------------
# MULTISEG-003: per-network TUN lifecycle (attach_tun/detach_tun become
# per-network; each network creates/tears down its own OS TUN device)
# --------------------------------------------------------------------------

class PerNetworkTunLifecycle(Requirement):
    """REQUIREMENT-ID: MULTISEG-003

    Step 4 of the multi-segment TUN plan. `MeshManager::attach_tun`/
    `detach_tun` (the embedding API previously used once, daemon-wide, by a
    hypothetical mobile embedder attaching a single `VpnService` fd) now take
    a `network: &str` and operate on that network's own
    `peers`/`tun_name`/`tun_tx`/`tun_tasks` (all moved onto `NetworkHandle` by
    MULTISEG-002). **New finding since the doc was written:** `ray-mobile`
    was removed by MINIMAL-016 and grepping the workspace `Cargo.toml` and
    `src/` finds no in-tree consumer of this embedding API today — extending
    it to be per-network (called once per network instead of once per daemon)
    is a natural extension, not a break of any live integration. A future
    embedder attaches one packet interface per network it wants active,
    rather than one for the whole daemon.

    `run_daemon` (`bootstrap.rs`) no longer creates one OS TUN device at boot
    before any network exists. Instead, `MeshManager::
    create_and_attach_network_tun(network, my_ip, subnet)` runs inside each
    of the three live network-establishment paths (`create_network_inner`,
    `finalize_join`, `restore_coordinator_network`), right after that
    network's `NetworkHandle` is inserted: it calls `tun::create()` in that
    network's own subnet, records the OS-assigned device name (already unique
    per call — `tun.rs`'s Step-0 finding that every function is already
    parameterized by device name held up unchanged), and calls the new
    per-network `attach_tun`. Failure is non-fatal (logged, network stays
    control-plane-connected without a data plane), matching `activate()`'s
    existing warn-don't-fail pattern for TUN problems.

    If the VPN is already active (`self.active`) at that point —
    `tetron join`/`create` while already up, or a restore whose attach lands
    after boot's one `activate(None)` call already ran — this also brings
    that network's link up and installs its routes immediately, instead of
    waiting for a future `activate()` call it would otherwise miss entirely
    (since `activate()` only iterates whatever is in `self.networks` *at the
    moment it runs*). **Known, documented, unclosed residual race:**
    `connect_all_networks` fires each saved network's restore as a detached
    `tokio::spawn` task and does not await them; in principle a restore's own
    post-attach `self.active` check could run a moment before `activate()`'s
    own `self.active.swap(true, ...)` executes, in which case neither catches
    it and that network's TUN stays administratively down until a manual
    `tetron down && tetron up`. In practice every restore does a DHT
    round-trip (tens to hundreds of ms) before reaching that check, while
    `activate()`'s swap runs within microseconds of `connect_all_networks()`
    returning, so the window is not expected to be hit — but it is not a hard
    guarantee, and closing it fully would mean awaiting every restore before
    `run_daemon` proceeds, undoing the fire-and-forget design
    `connect_all_networks` deliberately uses so one dead/slow network can't
    delay the others (a DIAL-001-adjacent tradeoff this does not reopen).
    Documented in code at `MeshManager::create_and_attach_network_tun`'s doc
    comment; flagged here as a known gap needing live multi-network-boot
    testing to actually observe (or not) before this can be fully trusted.

    `activate()`/`deactivate()` (previously operating on one daemon-wide
    `tun_name`) now iterate `self.networks`, bringing every network's own link
    up/down and installing its own loopback self-route (`handle.my_ip`, which
    MULTISEG-004 makes genuinely per-network rather than the node-wide
    identity IP). **Known, documented, unresolved limitation surfaced by this
    change, not present in the original scoping doc:** peer IPv6 addresses
    (`derive_ipv6`) are identity-derived and global across every network a
    node joins (`200::/7`, "never rotates" per `AGENTS.md`'s addressing
    section) — unlike IPv4, they are not subnet-scoped per network. The
    `route_peer_range` call installs one system-wide `200::/7 -> <tun>`
    kernel route; with N TUN devices the last one activated would otherwise
    silently win that route, leaving every other network's peers unreachable
    over IPv6 (IPv4 stays correctly segmented regardless, since each network
    has its own distinct v4 subnet/TUN). `activate()` now installs the
    `200::/7` route on only the first network encountered, deterministically,
    so this is an explicit "IPv6 mesh reachability works on one segment only"
    limitation rather than a silent last-writer-wins race. This is a genuine,
    unresolved product question (does multi-segment TUN need IPv6 addressing
    to become per-network too, e.g. by deriving it from `(identity, network)`
    the way IPv4 already is, or is single-segment IPv6 an acceptable interim
    state?) that was out of scope to resolve in this pass and needs an
    explicit decision before this ships.

    Network teardown (`teardown_network_runtime`, reached by `leave_network`/
    `nuke_network`'s solo-coordinator immediate-destroy path/kick-of-self)
    now aborts that network's own forwarding tasks and calls the new
    `tun::delete()` (added to `src/tun.rs`: `ip link delete` on Linux,
    `ifconfig <name> destroy` on macOS) rather than relying solely on the
    kernel to reclaim the device whenever the whole process eventually exits.
    This incidentally closes the pre-existing "stale TUN devices survive a
    daemon restart/crash" gap logged in the ideas doc's "Fallback" section —
    per-network teardown now runs mid-process, with other networks' devices
    still live, so relying on process-exit-triggered cleanup was no longer
    viable regardless.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/004/005/006.
    """
    req_id = "MULTISEG-003"


# --------------------------------------------------------------------------
# MULTISEG-004: relax SUBNET-010 (per-network TUN means no shared TUN left
# for a subnet mismatch to break); SUBNET-014's warning mechanism retired
# --------------------------------------------------------------------------

class SubnetCoherenceRelaxed(Requirement):
    """REQUIREMENT-ID: MULTISEG-004

    Step 2 of the multi-segment TUN plan, landed only once MULTISEG-003 (per-
    network TUN) actually exists — see the corrected "Suggested commit
    sequence" logged in `DO-NOT-COMMIT/IDEAS_MultiSegmentTUN.md`: relaxing
    this before per-network TUN existed would have reintroduced
    SUBNET-BUG-001 (joining a network whose subnet didn't match the single
    shared TUN silently misconfigured that TUN, breaking the data plane with
    no error). Per-network TUN removes the precondition that bug depended on
    — there is no longer a single shared TUN for a network's subnet to
    disagree with.

    **Create side** (`create_network_inner`): removed SUBNET-010's rejection
    of a `--subnet` that disagreed with the already-persisted node-wide
    value. A brand-new network name has nothing to conflict with (the
    existing `already active` check already rejects reusing a name); the only
    remaining validation is the pre-existing `already active`/hostname/CIDR
    checks. `AppConfig.subnet` (the node-wide cache, `config::node_subnet()`)
    keeps exactly one job: seeding the *default* subnet for a create with no
    explicit `--subnet` and nothing persisted yet. An explicit `--subnet`
    still updates that default (for the node's next unspecified create), it
    just no longer gets rejected for disagreeing with a prior one.

    **Join side** (`join_network_inner`): removed the SUBNET-BUG-001 guard
    outright (`network_subnet != node_subnet` -> `bail!`). `my_ip` is now
    derived directly from the joining network's own blob-carried subnet
    (`if network_subnet == self.identity.subnet() { self.identity.local_ip() }
    else { derive_ip(&self.identity.local_identity(), network_subnet) }`),
    mirroring the derive-if-different pattern `create_network_inner` already
    used — this was a real, previously-missed bug in the pre-relaxation code:
    `my_ip` was computed from `self.identity.local_ip()` (the node-wide
    identity IP) unconditionally, which would have been wrong the moment the
    coherence guard was removed without this fix. `restore_coordinator_network`
    gets the equivalent fix: its `subnet`/`my_ip` are now derived from
    `NetworkConfig.subnet` (MULTISEG-001's field — this is its first real
    read) falling back to the default, not from `self.identity.subnet()`.

    **SUBNET-014's warning mechanism is retired, not removed.** That
    requirement's `warning: Option<String>` field on the `Created`/`Joined`
    IPC responses existed because a subnet mismatch used to require a full
    `sudo tetron restart` to take effect on the one shared TUN. That scenario
    no longer exists — every network's TUN is created fresh, in its own
    correct subnet, at the moment it's established. All four call sites that
    used to call `membership::subnet_change_warning` now pass `warning: None`
    unconditionally. The wire field itself, `subnet_change_warning`'s
    definition, and its unit test are left in place (harmless, no longer
    exercised by any live call site) rather than removed — deleting an
    IPC/wire surface is a separate, deliberate cleanup better done on its own,
    not a drive-by of this change. Flagged as a real follow-up, not forgotten.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/003/005/006.
    """
    req_id = "MULTISEG-004"


# --------------------------------------------------------------------------
# MULTISEG-005: forward.rs needs no changes -- confirmed, not just assumed
# --------------------------------------------------------------------------

class ForwardingLoopUnchanged(Requirement):
    """REQUIREMENT-ID: MULTISEG-005

    Step 5 of the multi-segment TUN plan. Confirms (rather than merely
    assumes, per the ideas doc's own "Still unverified" caveat) that
    `src/forward.rs` needed zero code changes. `run_mesh`, `spawn_tun_writer`,
    `spawn_peer_reader`, and `ForwardCtx` already took `peers`/`tun_tx` as
    plain parameters/fields with no daemon-wide assumption baked into their
    own bodies — the daemon-wide-ness lived entirely in what
    `MeshManager::attach_tun` passed them, not in `forward.rs` itself. Once
    MULTISEG-002/003 made that a per-network `peers`/`tun_tx` pair, the same
    loop runs once per network's TUN reader task (mirroring the pre-existing
    one-writer/one-reader-task-per-`attach_tun`-call pattern, now called once
    per network instead of once per daemon) with no logic change. The
    per-packet ingress anti-spoof check (`evaluate_inbound`, a peer may only
    source packets from its own mesh IP) is unaffected: it validates a
    datagram against the specific peer that sent it, already scoped to one
    connection regardless of how many networks or tables exist elsewhere.

    Found: 2026-07-18, `feat/multi-segment-tun` branch, landed together with
    MULTISEG-002/003/004/006.
    """
    req_id = "MULTISEG-005"


# --------------------------------------------------------------------------
# MULTISEG-006: remaining daemon-wide `self.peers`/`self.mesh_ctx()` call
# sites updated to their network-scoped equivalents
# --------------------------------------------------------------------------

class RemainingPeerTableCallSitesScoped(Requirement):
    """REQUIREMENT-ID: MULTISEG-006

    Step 6 of the multi-segment TUN plan. Beyond the sites the ideas doc
    enumerated (`accept.rs`'s `self.ctx.peers.add(...)`, already
    network-scoped via the `MeshCtx` each accept handler already carries as a
    struct field — needed zero changes, confirmed; `runtime.rs`'s
    `leave_network`/`kick_member`; `create_join.rs`'s create/join paths,
    covered by MULTISEG-002/003/004 directly), a full-crate re-grep (not
    trusting the doc's now-stale line numbers) found three more daemon-wide
    `self.peers`/`self.mesh_ctx()` sites the doc's Step-6 pass had not
    enumerated: `daemon/mesh/admin.rs`'s `admin_add` (finding the live
    connection to send an `AdminGrant` over), `daemon/mesh/publish.rs`'s
    `store_and_publish_group` (collecting seed peers for a re-publish after
    `tetron accept`-style admission), and `daemon/mesh/diagnostics.rs`'s
    `network_status` (building `tetron status`'s per-peer connection info).
    All three now resolve `self.networks.get(network)` and read that handle's
    own `peers` table instead of a daemon-wide one. `runtime.rs`'s
    `leave_network` (closing connections gracefully before teardown) and
    `kick_member` (via `mesh_ctx_for(network)` replacing `self.mesh_ctx()`)
    were fixed as part of the same sweep. None of these needed the
    `_for_network`/`_by_network`-suffixed `PeerTable` methods themselves to
    change (see MULTISEG-002's note on why `peers.rs` has zero code changes)
    — only which table instance each call site reads.

    Found: 2026-07-18, `feat/multi-segment-tun` branch (the three
    previously-unenumerated sites found via `cargo build` after
    MULTISEG-002/003/004 landed, not via grep — the compiler caught what a
    line-based grep across multi-line `self\n    .peers` call chains missed).
    Landed together with MULTISEG-002/003/004/005.
    """
    req_id = "MULTISEG-006"


# --------------------------------------------------------------------------
# MULTISEG-007: join-side anti-spoof false positive on a subnet-diverging
# network -- found via live 3-machine testing, not caught by reconcile.py
# --------------------------------------------------------------------------

class JoinSideIpDerivationFixed(Requirement):
    """REQUIREMENT-ID: MULTISEG-007

    Found live-testing MULTISEG-002..006 on 3 real machines (aorus
    coordinating two networks at once: `multiseg-test-a` on the node-wide
    default subnet, `multiseg-test-b` on an explicit `--subnet 10.77.0.0/16`
    diverging from it) — `reconcile.py` was green throughout MULTISEG-002..006
    and never caught this; it is a real functional bug, not a lint/build gap.

    **Symptom:** every real packet from the coordinator (aorus) to a member
    (x10sra) on the subnet-diverging network was silently dropped by the
    member as `DropSpoof` ("dropped inbound packet with spoofed source IP"),
    100% loss, while the identical topology on the network sharing the node's
    default subnet worked fine (`multiseg-test-a`, aorus<->xps). The QUIC
    control connection itself was healthy (`tetron status` showed a live,
    connected peer on both sides) — only the data-plane anti-spoof check
    (`forward::evaluate_inbound`, "a peer may only source packets from its
    own mesh IP") was failing.

    **Root cause:** `daemon/mesh/join.rs`'s `join_mesh_shared` computed both
    its own `my_ip` (`identity.local_ip()`) and the coordinator's `remote_ip`
    (`identity.derive_ip(&remote_id)`) via `IrohIdentityProvider`'s trait
    methods — bound to the single subnet baked into `MeshCtx.identity` at
    daemon boot (the node-wide default, `config::node_subnet()`), which
    MULTISEG-002's per-network `MeshCtx` restructuring left unchanged (every
    network's `MeshCtx` clones the same node-wide-subnet identity provider;
    only `peers`/`tun_tx` became per-network). `create_network_inner`
    (create_join.rs) and `join_network_inner`'s own subnet resolution
    (create_join.rs, MULTISEG-004) both correctly derive a network-scoped
    `my_ip` via the free `membership::derive_ip(identity, network_subnet)`
    function when the network's subnet differs from the identity's default —
    but that correct value never reached `join_mesh_shared`, because
    `JoinParams` (the struct threading per-join inputs into it) had no
    `my_ip` field at all, so `join_mesh_shared` recomputed its own (wrong)
    value from scratch. The `remote_ip` used to register the coordinator's
    connection for the anti-spoof check (`register_mesh_peer` ->
    `spawn_peer_reader`'s `peer_ip` parameter) was `identity.derive_ip`
    against the same wrong node-wide subnet, landing outside the network's
    real range — so a legitimate packet correctly sourced from the
    coordinator's real (correctly-derived, roster-authoritative) IP failed
    the `src_ip == expected_peer_ip` check every time.

    A second, lower-severity effect of the same root cause: `my_ip` also fed
    `persist_join_config`, so the wrong value was written to
    `NetworkConfig.my_ip` on disk for a subnet-diverging network — the live
    in-memory value (correctly set elsewhere, from `JoinContext.my_ip`) papered
    over this at runtime, but a fresh restart reading the persisted value back
    could have surfaced it. Fixed by the same change.

    **Fix:** `JoinParams` gained a `my_ip: Ipv4Addr` field; `run_join_handshake`
    (create_join.rs) now passes `ctx.my_ip` (the already-correct,
    network-scoped value) through instead of `join_mesh_shared` recomputing
    it. `remote_ip` is now looked up from the just-admitted `members` roster
    returned by `perform_join_handshake` (authoritative, network-scoped, and
    already available at that point in the function) rather than
    re-derived; `identity.derive_ip(&remote_id)` is kept only as a defensive
    fallback for the practically-impossible case of the coordinator not
    being in its own roster.

    **Not fixed, found harmless on inspection:** `accept.rs`'s
    `handle_connection` has an analogous-looking fallback
    (`member_ip.unwrap_or_else(|| self.ctx.identity.derive_ip(&remote_id))`)
    for a *fresh, not-yet-admitted* joiner. Traced its only consumer,
    `admit_peer`'s `_suggested_ip` parameter — the leading underscore was
    already a deliberate signal it's unused; `validate_admission` always
    recomputes the authoritative IP via `membership::assign_ip(&s.members,
    &remote_id, s.subnet)` (correctly network-scoped, since `s.subnet` is
    `NetworkState`'s own per-network field), and that value — not the
    fallback — is what actually gets registered. Left as-is rather than
    changed as a drive-by; a real (if confusing) piece of dead input, not a
    functional bug.

    Live-verified after the fix, same 3-machine topology: 0% loss both
    directions on the subnet-diverging network, `reconcile.py` green (build,
    clippy 0 warnings, tests, all identity/regression gates).

    Found: 2026-07-18, `feat/multi-segment-tun` branch, live 3-machine
    testing (aorus/xps-17-9720/x10sra) per `DO-NOT-COMMIT/TESTING.md`'s
    "multi-segment TUN" run.
    """
    req_id = "MULTISEG-007"


# --------------------------------------------------------------------------
# IPV6-001..003: per-network IPv6 addressing, the follow-up MULTISEG-003
# explicitly deferred ("Making IPv6 fully per-network would mean ... a
# larger, separate change")
# --------------------------------------------------------------------------

class PerNetworkIpv6Derivation(Requirement):
    """REQUIREMENT-ID: IPV6-001

    Follow-up to MULTISEG-003's flagged limitation: `derive_ipv6(identity)`
    is identity-only, so a node's peer IPv6 address is identical across
    every network it joins — unlike IPv4, which is genuinely per-network
    (`derive_ip(identity, subnet)`). This makes `derive_ipv6` take the
    network's own public key too, mirroring the IPv4 shape, so each
    network's v6 range becomes its own real, disjoint, routable block
    instead of one address shared across every network a node belongs to.

    **New signature:** `derive_ipv6(identity: &EndpointId, network: &EndpointId)
    -> Ipv6Addr`.

    **Structural split (decided 2026-07-18, not just "shrink the hash"):**
    byte 0 fixed `0x02` (unchanged, keeps the address inside the existing
    `200::/7` product-documented range) + a 48-bit **network-prefix**
    (bytes 1-6, `blake3(network.to_string())` truncated to 6 bytes) + a
    72-bit **peer-part** (bytes 7-15, `blake3(format!("{identity}:{network}"))`
    truncated to 9 bytes). The network-prefix is the part that actually
    matters: it is *only* a function of the network's public key, so every
    member of a given network shares the same 56-bit prefix (`0x02` + 48
    bits), giving that network a real `/56` CIDR block a route can target —
    without this structural split, folding "network" into the hash input
    alone would still produce addresses fully interleaved with every other
    network's, with no CIDR block to route (this is what IPV6-003 needs).
    The peer-part deliberately mixes in `network`, not just `identity`, so
    the same identity gets an unrelated peer-part in each network it joins
    — this closes a cross-network grinding-reuse loophole that would
    otherwise undermine IPV6-002's collision defense (see that
    requirement).

    **No collision-index** (confirmed 2026-07-18 via birthday-paradox math
    at the more realistic 1%-probability threshold, not just 50%): IPv4's
    default `/24` needs only ~2-3 nodes for a 1% collision chance (why
    `collision_index`/`assign_ip`'s rotation exists at all), while a
    72-bit peer-part needs ~3.1 billion nodes for the same 1% risk —
    astronomically beyond any realistic mesh size. `assign_ip`'s
    IPv4-style rotate-on-collision approach is not extended to v6; a
    genuine (non-adversarial) collision is not expected to ever occur.
    Deliberate grinding is a different threat model, handled separately by
    IPV6-002.

    **Call-site audit** (every non-test caller of the old identity-only
    signature, found via full-crate grep, each now threads through the
    relevant network's own public key — already in scope at every site
    below via `NetworkState.network_public_key`, `NetworkHandle.network_key`,
    or an explicit `network`/`net_pubkey` parameter already being passed
    for other reasons):
    - `daemon/mesh/create_join.rs` — create/join success paths building
      `my_ipv6`/roster `ipv6` fields for IPC responses.
    - `daemon/mesh/accept.rs` — `spawn_admitted_member_tasks` and the two
      other sites registering a peer's v6 route for the anti-spoof-adjacent
      data plane.
    - `daemon/mesh/diagnostics.rs` — `tetron status`'s per-network,
      per-member v6 display.
    - `daemon/mesh/join.rs` — registering the coordinator's v6 on initial
      join.
    - `daemon/mesh/coordinator.rs`, `daemon/mesh/reconverge.rs` — peer
      removal/pruning, which must recompute the same network-scoped v6 that
      was used to register the peer, or the removal is a no-op key-miss.
    - `daemon/mesh/runtime.rs`, `daemon/mod.rs` — `activate()` and
      `create_and_attach_network_tun`'s own-address computation, feeding
      `route_self_loopback` (each network's loopback self-route must match
      that network's own derived v6, not one node-wide value — the same bug
      shape as MULTISEG-007 if left identity-only here).

    `src/peers.rs`'s `PeerTable` needs no structural change (its `v6:
    Arc<FastDashMap<Ipv6Addr, PeerEntry>>` is already a distinct instance
    per network since MULTISEG-002 gave every `NetworkHandle` its own
    `PeerTable`) — only what value each call site above computes as the key
    changes.

    Existing unit tests (`test_derive_ipv6_deterministic`,
    `test_derive_ipv6_in_200_range`, `test_derive_ipv6_different_identities_differ`)
    update for the new signature; new coverage added for same-identity
    producing different addresses across two networks, and the network-
    prefix being shared across different identities on the same network.

    Found: 2026-07-18, decided during a design discussion following the
    MULTISEG-002..007 merge; implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-001"


class Ipv6CollisionRejectedAtAdmission(Requirement):
    """REQUIREMENT-ID: IPV6-002

    Defense-in-depth alongside IPV6-001's structural collision-resistance:
    mirrors `validate_admission`'s existing IPv4 behavior (`accept.rs`,
    "IP collision: {ip} already assigned" — a different identity already
    holding a candidate's derived address is rejected, not silently
    admitted) for IPv6. Scoped explicitly against a *deliberately grinded*
    collision (an adversary generating on the order of 2^36 keypairs to
    force a specific 72-bit peer-part match is realistically feasible with
    modest hardware), not the accidental case — IPV6-001's math already
    makes accidental collision astronomically unlikely; this closes the
    much narrower gap that a probabilistic argument alone does not cover
    for an adversarial actor.

    Since `Member` carries no persisted `ipv6` field (v6 addresses are
    never transmitted or signed — always freshly re-derived locally by
    every node, confirmed by inspection of the `Member` struct), the check
    cannot look up a stored value. `validate_admission` instead recomputes
    `derive_ipv6(&m.identity, &s.network_public_key)` for every existing
    roster/approved entry and compares against the joiner's candidate
    address — an O(n) scan, cheap at realistic roster sizes, same shape as
    the existing hostname-collision scan a few lines above it in the same
    function.

    On a collision against a *different* identity, admission is rejected
    with `"IPv6 collision: {addr} already assigned"` (mirroring the v4
    message's wording) — the joiner's admission fails outright. Unlike
    IPv4, there is no collision-index to rotate to and retry (IPV6-001
    deliberately has none), so this is a hard denial, not a resolution
    step. A re-add of the *same* identity (e.g. a reconnect) is not a
    collision, matching `assign_ip`'s existing same-identity exemption.

    Found: 2026-07-18, decided as part of the same design discussion as
    IPV6-001 (explicit user decision to add this check rather than accept
    the residual grinding risk); implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-002"


class PerNetworkIpv6RouteInstallation(Requirement):
    """REQUIREMENT-ID: IPV6-003

    Closes the limitation MULTISEG-003 explicitly flagged and deferred:
    "IPv6 mesh reachability works on one segment only" — `activate()`
    previously installed one system-wide `200::/7 -> <tun>` kernel route,
    guarded by an `installed_peer_range_route` bool so only the *first*
    network encountered got it (a deterministic, documented limitation,
    not a last-writer-wins race, but still a real one: every other
    network's peers were unreachable over IPv6). This was only fixable
    once IPV6-001 existed — a single shared `200::/7` superset has no
    narrower per-network block a route could target; disjoint per-network
    `/56` prefixes do.

    `tun::route_peer_range` changes signature from `(tun_name: &str)` to
    take the specific prefix/width to install (the network's own `/56`
    block: `0x02` + IPV6-001's 48-bit network-prefix, peer-part bits
    zeroed) instead of the hardcoded `Ipv6Addr::new(0x0200, ..), 7`
    literal — both the Linux (netlink `RouteMessageBuilder`) and macOS
    (`route add -inet6`) implementations swap the constant for the passed-
    in value. A new `membership::ipv6_network_prefix(network: &EndpointId)
    -> Ipv6Addr` helper computes the zeroed-suffix prefix address from
    IPV6-001's derivation, reused by both call sites below and by tests.

    Both call sites — `daemon/mod.rs`'s `create_and_attach_network_tun`
    and `daemon/mesh/runtime.rs`'s `activate()` — drop their "only the
    first network" bookkeeping entirely and call `route_peer_range`
    unconditionally per network: routes no longer collide, since each
    network's `/56` is disjoint from every other's (birthday math on a
    48-bit space, IPV6-001). `route_self_loopback`'s own-address argument
    switches from `derive_ipv6(identity)` to the network-scoped
    `derive_ipv6(identity, network)` at both sites, closing the same bug
    shape MULTISEG-007 fixed for IPv4 (a node-wide value used somewhere
    that needed to be network-scoped) before it can ever manifest here.

    `AGENTS.md`'s multi-segment TUN section and MULTISEG-003's own spec
    docstring both documented "IPv6 mesh reachability works on one segment
    only" as a known, unresolved limitation needing a product decision —
    that decision was made (IPV6-001) and this requirement is what acts on
    it; both docs get their limitation note removed/updated to reflect the
    resolved state once this lands and is live-tested.

    Needs its own live multi-machine test: a node dual-homed on two
    networks reaching a peer over IPv6 on *both* networks simultaneously
    (the exact scenario MULTISEG-003 could not support), not just IPv4 as
    the earlier MULTISEG live-testing pass covered.

    Found: 2026-07-18, decided as part of the same design discussion as
    IPV6-001/002; implemented on `feat/ipv6-per-network`.
    """
    req_id = "IPV6-003"


# --------------------------------------------------------------------------
# MACOS-001: fix macOS route_peer_range's hardcoded pre-fork CGNAT literal
# --------------------------------------------------------------------------

class MacosRoutePeerRangeUsesActualSubnet(Requirement):
    """REQUIREMENT-ID: MACOS-001

    `src/tun.rs`'s macOS variant of `route_peer_range` (needed because
    macOS's point-to-point `utun` doesn't reliably self-install either
    range the way Linux's kernel does) hardcoded the pre-fork upstream
    literal `100.64.0.0/10` for the IPv4 family, regardless of the
    network's actual configured subnet. Since tetron's own default is
    `10.88.0.0/24` (SUBNET-011), this silently misrouted IPv4 on every
    macOS-joined network by default — the exact same bug shape as
    MULTISEG-007 (a hardcoded/wrong value used where a network-specific
    one was needed), just never caught because no macOS build/test has
    run in CI (`build-macos` is `if: false` in both `nightly.yml` and
    `release.yml`, specifically citing this bug as the reason it's
    gated off) or on real hardware since the bug was first found
    2026-07-17.

    **Fix:** `route_peer_range` (both the Linux and macOS `cfg` variants,
    which must share a signature) gained a `subnet: crate::membership::Subnet`
    parameter. The macOS body now formats `subnet` into a real CIDR string
    (`format!("{base}/{prefix}")`) and installs *that* as the `-inet` route
    instead of the literal. Linux's variant receives the same parameter
    (as `_subnet`, deliberately unused — the kernel already installs the
    correct IPv4 connected route from the interface's own address/netmask
    automatically on link-up, so Linux never needed this to begin with).
    Both call sites (`daemon/mod.rs`'s `create_and_attach_network_tun`,
    which already had the network's `Subnet` as its own parameter, and
    `daemon/mesh/runtime.rs`'s `activate()`, which reads it from
    `handle.state.read().unwrap().subnet`) now thread the real value
    through instead of the function inventing its own.

    **Not yet verified on real hardware or in CI** — found and fixed via
    direct code read (this bug cannot be exercised or caught by
    `reconcile.py`'s Linux-only build/test/clippy gates, same as
    MULTISEG-007 needed live multi-machine testing to surface). Real
    verification (native build + `sudo tetron up` + join an existing
    mesh + confirm IPv4 reachability, mirroring the live-testing rigor
    already applied to MULTISEG-002..007 and IPV6-001..003) is a
    separate, subsequent step on real Apple Silicon hardware — this
    commit is the code fix only. `build-macos`'s `if: false` should stay
    in place until that real-hardware pass actually happens; flipping it
    based on this fix alone (unverified) would repeat exactly the mistake
    the CI comment was written to prevent.

    Found: 2026-07-17 (original discovery, logged in
    `DO-NOT-COMMIT/TODO.md`'s "macOS port" section). Re-confirmed still
    present 2026-07-18 while auditing macOS support end to end. Fixed:
    2026-07-18.
    """
    req_id = "MACOS-001"


# --------------------------------------------------------------------------
# MACOS-002: capture real route(8) output instead of only its exit code
# --------------------------------------------------------------------------

class MacosRouteCommandOutputCaptured(Requirement):
    """REQUIREMENT-ID: MACOS-002

    Found live 2026-07-18 diagnosing `MACOS-001` on real Apple Silicon
    hardware: even after that fix, a `tetron down` / `tetron up` cycle
    left the network's IPv4 peer route missing from the routing table,
    silently breaking outbound connectivity (inbound still worked — the
    forwarder's inbound path has no destination check and writes straight
    to the TUN regardless of routing, so only *outbound* traffic showed
    the symptom, and the daemon logged no error at all). Manually running
    the *exact same* `route -n add -inet -net <cidr> -interface <tun>`
    command as root, standalone, worked correctly and the route appeared.
    So the command is right; something about the daemon's own execution
    of it differs, and `route_peer_range`'s exit-code-only check
    (`.status()`, discarding stdout/stderr) couldn't distinguish "really
    succeeded" from "exited 0 but the OS didn't do what was asked" —
    there was no way to see what actually happened.

    **Fix (observability only, not a behavior fix):** `route_peer_range`'s
    macOS variant now uses `.output()` instead of `.status()` for both
    the pre-add `delete` and the `add` themselves, logging the real exit
    status, stdout, and stderr — `debug` level for the delete (failure
    there is normal, it's cleaning up a possibly-nonexistent stale route)
    and for a successful add, `warn` level with the full output on a
    failed add (replacing the old bare `anyhow::ensure!` that discarded
    whatever `route(8)` actually printed).

    **Deliberately scoped narrow**: this is the one code path currently
    being live-debugged, not a sweep of every `Command::new(...).status()`
    call in `tun.rs` (e.g. `route_self_loopback` has the identical
    blind-exit-code pattern and is not touched here) — logged as a
    follow-up, not done now, since widening scope here would slow down
    the actual diagnosis this exists to unblock.

    Found: 2026-07-18, live-debugging `MACOS-001` on real Apple Silicon
    hardware (M1 MacBook Pro) after the fix alone didn't restore
    connectivity across a down/up cycle. Root cause of *why* the daemon's
    own route add doesn't take effect is still open — this requirement
    only adds the visibility needed to find it.
    """
    req_id = "MACOS-002"


# --------------------------------------------------------------------------
# MULTISEG-008: member-side NetworkState subnet still defaulted to the
# node-wide subnet — one MULTISEG-004 call site the original sweep missed
# --------------------------------------------------------------------------

class MemberJoinNetworkStateSubnetFixed(Requirement):
    """REQUIREMENT-ID: MULTISEG-008

    `MACOS-002`'s new logging found the actual root cause behind
    `MACOS-001` still not restoring IPv4 connectivity across a `tetron
    down`/`up` cycle on macOS: `route_peer_range` was correctly threading
    through whatever subnet it was given, but the subnet it was *given*
    was wrong. `daemon/mesh/join.rs`'s `build_member_state` — the
    function that builds a joining/reconnecting **member**'s live
    `NetworkState` — still constructed it with `subnet:
    crate::config::node_subnet()` (the node-wide default), a leftover
    from before multi-segment TUN existed (its own comment said so
    explicitly: `"SUBNET-010: single-TUN node — subnet comes from the
    persisted node cache ... not the network record"`).

    Every *other* `NetworkState` construction site was updated during
    `MULTISEG-004`'s sweep to use the network's own resolved subnet
    instead (`create_network_inner`, `restore_coordinator_network`, the
    DHT-fallback and try-fetch member paths in `create_join.rs`) — this
    one, reached only via the live member join/reconnect path
    (`join_mesh_shared` → `build_member_state`), was missed. Not
    macOS-specific at all: this is a data-model bug in the daemon's
    in-memory state, present on every platform. It went unnoticed until
    now for two independent reasons: (1) on Linux, IPv4's connected route
    is installed automatically by the kernel from the interface's own
    address/netmask — `route_peer_range`'s Linux variant never reads its
    `subnet` parameter at all, so a wrong `NetworkState.subnet` had no
    IPv4 symptom there; (2) IPv6's routing (`IPV6-003`) derives its
    prefix from `network_key`, never from `subnet`, so it was unaffected
    either way. `MACOS-001` was the first code path on any platform to
    actually *read* `NetworkState.subnet` for something user-visible
    outside of admission bookkeeping, which is what finally surfaced this.

    **Fix:** `JoinParams` gained a `network_subnet: crate::membership::Subnet`
    field, populated from `JoinContext.network_subnet` (already correctly
    resolved by the caller before dialing, per `MULTISEG-007`'s `my_ip`
    fix — the exact same pattern, same missing thread, same root cause
    class). `build_member_state` now takes `subnet` as a parameter
    instead of computing its own default.

    Found: 2026-07-18, live-debugging `MACOS-001`/`MACOS-002` on real
    Apple Silicon hardware (M1 MacBook Pro) — a `tetron down`/`up` cycle
    on a member of a subnet-diverging network installed a route for the
    *node's default* subnet instead of that network's actual one, so
    outbound IPv4 traffic had no working route (inbound still worked,
    since the forwarder's inbound path has no destination/routing
    dependency). Not yet re-verified live after this fix — that's the
    immediate next step, same M1 hardware, same reproduction (join a
    subnet-diverging network, `down`, `up`, confirm the route now matches
    the network's real subnet and IPv4 connectivity survives the cycle).
    """
    req_id = "MULTISEG-008"


# --------------------------------------------------------------------------
# MULTISEG-009: join-side collision false-positive -- same root-cause class
# as MULTISEG-007 (a locally-derived `my_ip` guess disagreeing with the
# roster), this time on the *collision* check rather than the anti-spoof
# check. Found live via two concurrent tetron-testsuite OOM-repro runs.
# --------------------------------------------------------------------------

class WelcomeAdoptsAuthoritativeSelfIp(Requirement):
    """REQUIREMENT-ID: MULTISEG-009

    Found live 2026-08-11: two separate `tetron-testsuite` OOM-repro test
    runs (`oom-repro-t3-churn.sh`'s node4, `oom-repro-t4-soak.sh`'s node1)
    joined the real `testing-delete-me` network within about a minute of
    each other. The second join failed outright:
    ```
    ! join failed
      no coordinator admitted the join (tried 2): IP collision: 10.77.0.101
      is already assigned to 1635d23e9f405688140605d84ef89334763ed7bb84468bef82d0d7938184ea3c
    ```
    against both of that network's coordinators identically, even though
    `src/addressing.rs::assign_ip`'s collision-index rotation exists
    specifically to resolve this exact case (its own doc comment: "two
    different identities hashed to the same virtual IP").

    **Root cause, traced end to end, not assumed:** the coordinator side
    was already correct. `accept.rs::validate_admission` calls `assign_ip`
    authoritatively and explicitly ignores the joiner-suggested IP
    (`admit_peer`'s `_suggested_ip` parameter, underscore-prefixed, unused
    — its own doc comment: "the lowest free collision index (not the
    peer-suggested address)"). So a coordinator admitting a colliding
    identity correctly bumps it to the next free index and puts that
    bumped IP in the `Welcome.members` roster it sends back.

    The bug was entirely client-side, in `join.rs::perform_join_handshake`'s
    `initial` branch (line ~555): `my_ip` is computed once, locally, before
    ever contacting a coordinator (`create_join.rs`'s index-0
    `derive_ip`, blind to the roster, same pre-dial-guess pattern
    `MULTISEG-007` already fixed for `remote_ip`). On `Welcome`,
    `select.rs::welcome_ip_collision` checked whether *that stale guess*
    belonged to a different identity in the fresh roster and bailed if so
    — discarding the fact that `Welcome.members` already contained the
    joiner's own correctly-resolved (possibly bumped-index) entry, sitting
    right there unused. The coordinator had already solved the collision;
    the client just never read its own answer out of the response.

    **Fix:** on `Welcome`, look up the joiner's own entry in `members` by
    identity first. If present, its `ip` is the authoritative `my_ip` —
    adopt it instead of bailing; `welcome_ip_collision` (kept, renamed
    conceptually to a defensive check) now only fires if that lookup is
    absent (a genuine anomaly: the coordinator claims to have admitted us
    but our own identity isn't in the roster it sent) or if the adopted IP
    is *still* held by someone else after the lookup (should not happen by
    `assign_ip`'s construction, but not assumed). No retry, no second
    round-trip against another coordinator — the already-successful
    admission is used as-is. `HandshakeOutcome::Admitted`/`JoinResult`
    thread the adopted IP back up to `join_network_inner`
    (create_join.rs), which now uses it (not the stale pre-dial guess) for
    `persist_join_config`, `spawn_roster_peer_dials`,
    `spawn_reconverge_worker`, `NetworkHandle.my_ip`, TUN creation, and the
    `Joined` IPC response — all of which, pre-fix, would have silently
    kept using the wrong (never-collided-in-the-first-place, so never
    triggered the bail, but also never the *bumped* address) value on the
    non-colliding path too, since the local guess and the roster value
    only ever agreed by coincidence at index 0.

    **Not in scope, tracked separately:** `dial_fresh_join`'s
    coordinator-retry loop reuses the same `GroupBlob` snapshot (`data`)
    across every coordinator attempt in `order` — real staleness for
    other rejection reasons (e.g. a revoked invite), but not the cause of
    this bug, since the fix above means a colliding join now succeeds on
    the first coordinator that admits it rather than needing a second
    attempt at all.

    Found: 2026-08-11/12, `tetron-testsuite` OOM-repro session
    (`DO-NOT-COMMIT/RESULTS_session-2026-08-11_oom-repro-and-connection-stability.md`
    bug #5 / `DO-NOT-COMMIT/TODO_DETAILS.md#concurrent-join-ip-collision`),
    live against the real `testing-delete-me` network, not synthetic.
    """
    req_id = "MULTISEG-009"


class SubnetDriftOnRestart(Requirement):
    """REQUIREMENT-ID: SUBNET-DRIFT-001

    Found live-testing `STATUS-002` on real hardware (2026-07-20): exposing
    a network's subnet in `tetron status` for the first time immediately
    surfaced that a real, long-running test network's two peers disagreed
    about their own shared network's subnet, and about each other's IP.
    Confirmed as an actual data-plane break, not cosmetic: the coordinator's
    real TUN device (`ip addr`) was on a completely different subnet than
    either peer's roster-recorded IP, and `ping` between them showed 100%
    loss both ways -- despite `tetron status` (both before and after
    `STATUS-002`) showing "direct" connectivity with real, non-zero byte
    counters, because that traffic was control-channel/QUIC-transport
    chatter, not application-level TUN-forwarded packets, which don't
    exercise the same code path at all.

    **Root cause, two independent bugs, one shared design flaw.** Both
    `NetworkConfig.subnet` (local per-network config) and `GroupBlob.subnet`
    (the signed, network-wide DHT record every peer trusts) used the same
    convention: `None` means "the compiled `default_subnet()`," kept so a
    default-subnet network's config/blob stays byte-identical
    (`MULTISEG-001`). This is lossy the moment a node's own subnet
    preference can differ from the compiled default *and* can drift
    independently over time (true since `MULTISEG-002..007` let each
    network keep an independent subnet) -- `None` can no longer distinguish
    "this network genuinely wants the compiled default" from "this network
    wants whatever the node's default happened to be back when it was
    created."

    1. **Coordinator restart** (`restore_coordinator_network`,
       `src/daemon/mesh/runtime.rs`): resolved a restored network's subnet as
       `net_config.subnet.unwrap_or_else(default_subnet)` -- falling back to
       the compiled constant whenever the local config's `subnet` was
       `None`, without ever consulting the node's actual current subnet
       setting, let alone the network's *original* one. A network created
       while the node's default was e.g. `10.77.0.0/24` (so `subnet: None`
       was correctly persisted at the time, matching what was then the
       node's default) gets silently repinned to the compiled
       `10.88.0.0/24` on every subsequent restart.
    2. **Member restart when the DHT/blob is transiently unreachable**
       (`fallback_blob_from_config`, `src/daemon/mesh/create_join.rs`):
       synthesized a fallback blob with `subnet: Some(config::node_subnet())`,
       reasoning (per its own comment) that this was "safe per the
       SUBNET-BUG-001 invariant: an already-joined member's node subnet
       already matches its network's." That invariant held only in the
       pre-multi-segment world where a node ran one shared TUN/subnet for
       everything; `AGENTS.md` itself documents that `SUBNET-010`'s
       node-wide coherence check was removed once each network could keep
       its own subnet. Worse, both `create_network_inner` and
       `finalize_join` *mutate* the node's global default subnet as a side
       effect of every create/join (`config::set_node_subnet`), so the
       invariant breaks the moment a node's *second* network uses a
       different subnet -- every previously-joined network relying on this
       fallback silently inherits whatever unrelated network was created or
       joined most recently.

    **Compounding, not just repeating:** `NetworkState.subnet`'s only path
    into the signed blob is `blob_subnet()` (`src/daemon/mod.rs`), which had
    the identical "`None` for the compiled default" collapse. So bug 1
    firing on a coordinator doesn't just corrupt that coordinator's own
    local state -- `seal_and_publish` immediately afterward republishes the
    now-wrong subnet into the canonical blob (as `None`, since the
    in-memory value now equals the compiled default), spreading the
    corruption to every peer that fetches the blob fresh afterward,
    independent of whether they hit bug 2 themselves.

    **Fix, three parts, per due-diligence discussion with Erik (chose
    "always persist explicitly" over an IP-address-inference self-heal,
    which would have to assume a prefix length the project's own history
    doesn't guarantee -- default_subnet() was `/16` before an earlier
    project-wide change to `/24`):**

    1. **Stop omitting the value, everywhere.** `blob_subnet()` now always
       returns `Some(self.subnet)`. `create_network_inner`'s config save,
       `restore_coordinator_network`'s config save, and `persist_join_config`
       (`src/daemon/mesh/join.rs`, which previously hardcoded `subnet: None`
       unconditionally on every fresh join) all persist the actual resolved
       subnet explicitly now, never conditionally collapsed. `fallback_blob_from_config`
       reads the network's own persisted `nc.subnet` instead of the
       unrelated node-wide `config::node_subnet()`. Removes the ambiguity at
       the source for anything created/joined/restored under this fix;
       self-heals a legacy `None` the first time it successfully restores,
       since the now-validated, correctly-resolved value gets persisted
       back explicitly.
    2. **Hard-fail instead of silently drifting, as a safety net independent
       of (1).** New `membership::validate_subnet_matches_roster(subnet,
       roster, self_identity)`: checks the resolved subnet against this
       identity's own already-signed roster IP (still reliably correct even
       when the top-level subnet cache has drifted, since member IPs are
       always persisted as absolute values, never conditionally omitted).
       A no-op if the identity isn't in the roster yet (a fresh join, not a
       restore -- nothing to check). Called in `restore_coordinator_network`
       (before `seal_and_publish`, so a bad resolution is never written back
       anywhere) and in `join_network_inner`'s restore/reconnect path, both
       returning a clear error naming both values instead of proceeding to
       attach a TUN that cannot route to any peer. Unit-tested directly
       (`validate_subnet_matches_roster_{ok_when_consistent,
       rejects_mismatch, ok_when_identity_absent}`, `src/membership.rs`).
    3. **Self-heal via IP inference: considered, not implemented.** Would
       need to assume a prefix length to back out a subnet from an existing
       roster IP alone, which isn't guaranteed sound given the project's own
       history (the compiled default itself changed prefix length once).
       (2) converts an already-corrupted network from "silently broken" to
       "loudly refuses to restore, names the inconsistency" -- sufficient
       without guessing. The one specific network found broken live-testing
       this is disposable test infrastructure; recreating it fresh (not a
       code change) is the pragmatic remediation for that specific instance.

    **Not yet re-verified live** after this fix (the bug was found via, but
    fixed after, the `STATUS-002` live-testing session) -- `cargo build`/
    `clippy`/`test` (220 tests, +3 new) and `reconcile.py` green. Redeploying
    to the same real hardware to confirm (2) actually catches the existing
    broken network and that a clean network's restart round-trips its
    subnet correctly is the natural next step.

    **Addendum, live-tested 2026-07-20:** verified end-to-end on real
    hardware (aorus, xps) exactly as planned above. Removed both machines'
    broken `systray-func-test` config (found along the way: `tetron leave`
    can't remove a network that failed to restore -- both its resolution
    paths only scan currently-*loaded* networks, never the full persisted
    config list; logged as a separate, low-urgency gap in
    `DO-NOT-COMMIT/TODO.md`, not fixed here), recreated it fresh, and
    confirmed: subnet persists identically on both sides across a create +
    join + restart cycle, real data-plane traffic (`ping`, then 20MB `scp`
    with a SHA-256 check) works with 0% loss, and it survives a second
    restart on both machines with no drift. Also joined a fourth machine
    (a MacBook Pro, Apple Silicon/macOS -- built natively there, synced
    from this repo directly over SSH rather than GitHub) to the same
    network with identical results, confirming the fix holds across
    architectures and operating systems, not just Linux x86_64.
    """
    req_id = "SUBNET-DRIFT-001"


class EachNetworkGetsADistinctSubnet(Requirement):
    """REQUIREMENT-ID: SUBNET-UNIQUE-001

    Found immediately during the `SUBNET-DRIFT-001` live-test follow-up:
    creating a second network on a node that already had one, without an
    explicit `--subnet`, silently gave it the *exact same* subnet as the
    first (`create_network_inner`'s unspecified-subnet path just resolves
    to the node's one persisted/compiled default, with no awareness of what
    other networks that same node already has). Concretely: the same node
    ended up with the identical address (`10.77.0.200`) on two supposedly-
    independent networks -- harmless given per-network TUN isolation
    (`MULTISEG-002..007`), but defeating a real purpose of configurable
    subnets, confusing to read, and a foreseeable source of firewall/
    routing-rule mistakes for anyone trying to distinguish networks by IP
    range. Erik: "MUST be a new subnet, always."

    **Fix:** new `membership::next_available_subnet(candidate, existing)` --
    given a starting candidate and every subnet already in use, advances by
    one full block (`2^(32-prefix)` addresses) per collision, prefix length
    fixed, until it finds one that overlaps nothing in `existing` (capped at
    4096 attempts, far beyond any real node's network count, after which it
    gives up and returns the last candidate rather than looping forever).
    Wired into `create_network_inner` (`src/daemon/mesh/create_join.rs`):

    - **No explicit `--subnet`** (the common path): the resolved default
      candidate is silently advanced past any collision with an existing
      network's persisted subnet (`config::load()?.networks[].subnet` --
      always populated now thanks to `SUBNET-DRIFT-001`'s "persist
      explicitly, never omit" fix, so this list is reliable). "Silently" as
      far as the resolution logic goes, but never silent to the *caller*:
      `IpcMessage::Created` gained a `subnet: String` field (both
      `create_network_inner` and `restore_coordinator_network`'s success
      responses), and `tetron create`'s own CLI output now prints a
      `subnet <cidr>` line unconditionally -- the actual chosen value is
      always visible, whether or not it's the one a caller might have
      expected.
    - **Explicit `--subnet`**: honored exactly, never silently substituted
      -- but rejected outright with a clear error if it collides with a
      network this node already has. An explicit request deserves a
      correction, not a silent override to something else.

    Unit-tested directly (`next_available_subnet_returns_candidate_when_free`,
    `_advances_past_one_collision`, `_advances_past_several_collisions_in_order`,
    `_keeps_prefix_length`, `src/membership.rs`) -- the "several collisions
    in order" case exercises exactly the reported scenario (candidate
    already taken, verifies it lands on the correct next free block, not
    just *some* free block).

    **Bundled discovery while fixing this, unrelated to the feature
    itself:** several existing unit tests across `src/membership.rs`,
    `src/config.rs`, `src/control.rs`, `src/packet.rs`, `src/peers.rs`, and
    `src/forward.rs` used `100.64.x.x` (the pre-fork default subnet,
    inherited from upstream and never updated after this fork changed the
    default to `10.88.0.0/24`) as test-fixture data. Most were harmless --
    arbitrary placeholder IPs where the specific value never mattered to
    what was being tested -- but three were genuinely **vacuous**, passing
    for an unintended reason rather than testing what they claimed to:
    `test_derive_ip_avoids_reserved` compared derived IPs (always inside
    `10.88.0.0/24`) against the *wrong* subnet's reserved addresses, so the
    assertion was vacuously true for every input, testing nothing;
    `validate_member_rejects_mismatched_ip`, `validate_member_rejects_
    reserved_addresses`, `validate_approved_rejects_mismatched_ip`, and two
    `decode_group_blob_rejects_*` tests all used out-of-`10.88.0.0/24`
    addresses where an *in-range* one was needed to actually exercise the
    specific rule each test was named for (mismatch / reserved-address
    rejection), instead accidentally passing via the unrelated out-of-range
    check every time. Fixed all of these to use addresses actually inside
    `default_subnet()`; mechanically swapped the remaining, genuinely
    arbitrary occurrences to `10.88.x.x` for consistency (`sed`-scoped to
    each file's test module, verified by rerunning every affected module's
    tests before and after). Left untouched: doc comments/prose correctly
    citing the real Tailscale range, and the two tests in `membership.rs`
    that deliberately compare against `100.64.0.0/10` on purpose
    (`subnets_overlap_detects_both_directions_but_not_disjoint`,
    `ensure_in_range_respects_custom_subnet`) -- changing either of those
    would have broken the actual thing they're testing.
    """
    req_id = "SUBNET-UNIQUE-001"


# --------------------------------------------------------------------------
# MTU-DIAG-001: surface MTU/fragmentation diagnostics instead of raw logs
# --------------------------------------------------------------------------

class MtuFragmentationDiagnostics(Requirement):
    """REQUIREMENT-ID: MTU-DIAG-001

    Directly motivated by the live regression found and fixed the same day
    (see `Ipv4Fragmentation`/FRAG-001's second follow-up addendum): diagnosing
    that F-04's checksum-verify bug had silently disabled all IPv4
    fragmentation required manually SSHing into a live peer and grepping its
    rolling log file for `"cannot fragment"`/`"fragmenting oversized"` --
    nothing about fragmentation activity, drop reasons, or per-peer QUIC
    datagram-size ceilings was ever visible through `tetron status`. This
    closes that gap: the same class of bug should surface as a clear signal
    the next time, not require raw log archaeology.

    **Drop-reason granularity (`src/stats.rs`):** `DropReason` gains
    `FragmentationFailed`, applied at both of `forward.rs`'s "cannot
    fragment" `None` branches (IPv4 checksum/options-reject, IPv6
    envelope-too-small) -- previously indistinguishable from a generic QUIC
    `SendFailure`. `DropReason::ALL` grows to 6 entries accordingly.

    **Fragmentation-activity counters:** `ForwardMetrics` gains
    `fragmented_ipv4`/`fragmented_ipv6` counters (`record_fragmented_ipv4`/
    `record_fragmented_ipv6`), each incremented once per *original* oversized
    packet that successfully split (not once per wire fragment) -- so
    "is fragmentation actually happening on this daemon" becomes a queryable
    number instead of a debug-level log line nobody watches by default.

    **Per-peer datagram-size ceiling (`tetron-proto::ipc::ConnectionInfo`):**
    gains `max_datagram_size: Option<u64>`, populated live in
    `diagnostics::gather_conn_info` from `conn.max_datagram_size()` at
    status-query time (not a stored counter -- Quinn's DPLPMTUD ceiling
    changes over a connection's lifetime, so a fresh read is the only
    meaningful value). `None` on a connection that doesn't currently support
    QUIC datagrams at all, mirroring the `Option` `forward.rs` already
    handles the same way. Flows into `--json` automatically since
    `ConnectionInfo` already serializes wholesale; no plain-text change --
    matches `STATUS-002`'s existing precedent that per-peer connection-health
    detail (rtt/tx/rx/ipv6) is `--json`-only, keeping the default aligned
    table uncluttered.

    **Daemon-wide summary (`IpcMessage::StatusResponse`):** gains
    `drops: DropCounts` (one field per `DropReason` variant) and
    `fragmented_ipv4`/`fragmented_ipv6: u64`, all `#[serde(default)]` so an
    older daemon's response still decodes. `--json` surfaces all of it
    (`"drops": {...}`, `"fragmented_ipv4"`, `"fragmented_ipv6"` alongside the
    existing `"traffic"` object). The plain-text view gains exactly one new
    line, shown only when at least one drop has occurred (matching the
    existing convention of omitting empty sections -- "no active networks",
    nuke-proposals only when non-empty): `drops N total (reason count, ...)`
    listing only the non-zero reasons by name, directly under the existing
    `traffic` line. A perfectly healthy daemon's default output is unchanged
    byte-for-byte.

    **Explicitly out of scope for this pass** (the TODO item's own
    "diagnostic tooling" bullet also floated an active MTU black-hole probe):
    no new active probing mechanism is added here -- this is purely surfacing
    state the daemon already computes internally on every packet. An active
    probe (deliberately sending oversized test traffic and confirming
    round-trip / PMTU convergence) is a separate, larger feature and remains
    unbuilt.
    """
    req_id = "MTU-DIAG-001"


# --------------------------------------------------------------------------
# Subnet collision prevention (SUBNET-COLLISION-*)
# --------------------------------------------------------------------------

class JoinSubnetCollisionCheck(Requirement):
    """REQUIREMENT-ID: SUBNET-COLLISION-001

    `tetron join` gains the same subnet-overlap check `tetron create`'s
    explicit `--subnet` path already has (`membership::subnets_overlap`
    against every subnet this node's other saved networks already use) --
    applied to the network's own blob-carried subnet, resolved in
    `join_network_inner` right after `network_subnet` is computed, for a
    fresh join only (`initial == true`; a boot-time restore of an
    already-joined network must never start failing this check
    retroactively -- there is no way to "fix" a years-old install's
    subnet choice from a restore path, and refusing to restore an existing
    membership would be far worse than the collision itself).

    Both this new join-side check and `create`'s existing explicit-`--subnet`
    check gain a `--force` flag to override -- today `create`'s check is an
    unconditional hard failure with no escape hatch at all, which is
    inconsistent with every other destructive/risky-state guard in this
    fork (`leave --force`, `nuke --force`) already following the
    reject-by-default-with-override pattern.

    Why this matters beyond the immediate footgun `DO-NOT-COMMIT/
    SUBNET_COLLISION.md` originally documented (two of a node's own
    networks silently sharing one subnet, written before this fork's
    spec-first workflow existed): it is also the one honest limitation
    flagged in PATH-BLEED-001's status-layer fix (PATHBLEED-STATUS-001) --
    a node joined to two networks with an overlapping subnet is exactly
    the case a pure subnet-boundary heuristic cannot distinguish from a
    genuine cross-network path bleed. Preventing the shared-subnet state
    at the source (this requirement) is more robust than working around
    it downstream (PATHBLEED-STATUS-001), and should land first.
    """
    req_id = "SUBNET-COLLISION-001"


class SubnetCollisionForcePhysicalLan(Requirement):
    """REQUIREMENT-ID: SUBNET-COLLISION-002

    Completes SUBNET-012 rather than duplicating it. `tun::check_subnet_overlap`
    / `local_ipv4_interfaces` (added for SUBNET-012) already shell out to `ip
    -o -4 addr show` to catch the overlay subnet colliding with a real local
    interface -- but only once, at daemon bootstrap, against the node-wide
    default subnet (`config::node_subnet()`). Two gaps closed here:

    1. **Wrong trigger point.** A specific network's subnet, resolved at
       `create`/`join` time (an explicit `--subnet`, or a joined network's
       own blob-carried subnet), is not necessarily the current node
       default -- so a colliding choice can go undetected until the next
       daemon restart happens to check whatever the default is by then.
       `local_ipv4_interfaces` is exposed for reuse and the same overlap
       check now also runs at `create`/`join` time against the specific
       resolved subnet, gated by the same `--force` flag SUBNET-COLLISION-001
       introduces (one flag, two checks: another tetron network, or the
       physical LAN).
    2. **macOS gap, pre-existing, not introduced here.** `local_ipv4_interfaces`
       has no `#[cfg(target_os = "macos")]` branch -- it unconditionally
       shells out to the Linux-only `ip` binary, which does not exist on
       macOS, so the function's own "fail-open if `ip` is unavailable"
       design means SUBNET-012 silently does nothing on macOS today. Fixed
       by adding a macOS branch (parsing `ifconfig` output), matching the
       per-OS-branch pattern every other multi-platform function in
       `src/tun.rs` already follows for the equivalent BSD-tool
       alternative.

    **Addendum, 2026-07-30: Android gap.** `find_physical_interface_collision`
    is `#[cfg(not(target_os = "android"))]` in `src/tun.rs` (Android has no
    `ip addr`/`ifconfig` to shell out to, and no embedder-supplied
    equivalent exists), but both call sites in
    `src/daemon/mesh/create_join.rs` (`create_network_inner`'s and the join
    path's physical-LAN checks) were unconditional -- a real build break on
    that target invisible to host `cargo check`, which never compiles under
    `target_os = "android"`. Found live cross-compiling a first Android
    embedder with `cargo ndk` (`E0425: cannot find function
    find_physical_interface_collision`). Fixed by gating both call sites
    `#[cfg(not(target_os = "android"))]` too, so the check is skipped
    entirely on Android -- consistent with the function's own documented
    fail-open behavior for a platform whose enumeration tool is
    unavailable, and with the existing `#[cfg(target_os = "android")]`
    guards already present elsewhere in `create_join.rs`/`runtime.rs`/
    `bootstrap.rs`/`tun.rs`/`config.rs`/`daemon/mod.rs` for the same class
    of desktop-only, embedder-inapplicable logic.
    """
    req_id = "SUBNET-COLLISION-002"
