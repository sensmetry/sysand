# PR #288 review follow-ups

Tracking list of review comments on [PR #288](https://github.com/sensmetry/sysand/pull/288)
(`breaking: reading based on new index structure`).

Check items off as they are addressed. Each item links to the original
GitHub discussion so the full context / diff hunk can be recovered.

Reviewers: `andrius-puksta-sensmetry` (A), `Jonas-Puksta-Sensmetry` (J),
`consideRatio` (E).

## Legend / conventions

- `- [x] …` — addressed in this branch (either by code/doc change or by
  an intentional decline).
- `- [x] … **Final check: confirmed resolved.**` — the item has been
  re-read in a second pass and confirmed done.
- `- [ ] … **Pending more review.**` — previously ticked but put back
  on the todo list during a second pass; the parenthetical records
  what was done before, so pick up from there.
- `- [ ] … **Reopened …**` — intentionally reverted from `[x]` to
  `[ ]` because a new design direction was chosen; the parenthetical
  records the current direction.
- Coordinated items: if two items share a rationale (e.g. the
  optional-digest question appears in `lock.rs` L580–581,
  `project/any.rs` L99, and the self-review "RemoteKpar checksum"
  item), the parentheticals cross-reference each other — changing one
  usually implies revisiting the others.

## Open items as of last walk-through

Eight items remain open; they cluster into three groups.

**Design / type-level digest requirement** (related, decide together):

- `core/src/lock.rs` L580–581 — reopened; considering an `IndexKpar`
  vs direct-HTTP split so the index path has a required digest at the
  type level.
- `core/src/project/any.rs` L99 — reopened alongside the above.

**`core/src/env/iri_normalize.rs`** (three adjacent items):

- L34 — return `Iri<String>` / IRI-type clarity; pending closer look.
- L48 — declined in favor of the known-valid splice over
  `iri::builder()`; GitHub reply still to post.
- L80 — "why reference?"; ambiguous — clarifying reply on GitHub
  needed before resolving.

**Other:**

- `core/src/env/discovery.rs` L4–17 — architectural question about a
  uv-style user-configured download/publish URL design; deferred.
- `core/src/project/utils.rs` L192–197 — cross-device rename inlined
  into `wrapfs::rename`; pending a more careful second review.
- `core/src/resolve/combined.rs` L169 — comment rewrite around
  I/O-vs-digest failure; pending a more careful second review.

---

## PR-level discussion

### From consideRatio (self-review, [comment](https://github.com/sensmetry/sysand/pull/288#issuecomment-4302695584))

- [x] **Timestamp details** — change is out of scope for this PR but
      initially bundled. Decide whether to split it out or keep the
      scope-bundling note in the PR body. (Addressed in this round:
      introduced `model::format_created` as the single producer of the
      `created` RFC-3339 string, switched precision to `Secs`, and
      routed every prod + test fixture through it. No direct
      `to_rfc3339_opts`/`SecondsFormat` uses remain.)
      **Final check: confirmed resolved.**
- [x] **Well-known discovery** — requesting `.well-known` from
      `/something/.well-known` (not the domain root) is unexpected.
      Ideas: avoid branching logic; don't use `.well-known`, use a
      direct name instead. **Conclusion: go for
      `/sysand-index-config.json`.** (Already addressed in commit
      ed54557.)
      **Final check: confirmed resolved.**
- [x] **RemoteKpar checksum** — should `LocalKpar` also carry a
      checksum? Re-evaluate symmetry between the two variants.
      (Declined on the same grounds as `lock.rs` L580-581: `Option` is
      load-bearing; a locally-declared kpar path doesn't inherently
      know its digest, and forcing hash-at-construction doesn't buy
      integrity the indexed path doesn't already provide.)
      **Final check: confirmed resolved.**
- [x] **Quality gates** — the phrase "well-formed archives" is used;
      make sure it is up to date with our actual requirements. (Added
      a §15 paragraph spelling out the split: wire-level rules are
      enforced by index CLIs / clients; semantic project-quality
      checks live in the publish pipeline. The "Well-formed archives"
      bullet now simply states that the full criteria are not frozen
      in v0 and are expected to evolve alongside the `sysand index`
      CLI, with a cross-reference to §15 for the wire-level vs.
      publish-time split — no concrete minimums are committed at this
      point.)
      **Final check: confirmed resolved — not requiring "well-formed
      archives" to be fully defined yet.**
- [x] **Index server management intent** — state explicitly that we
      only support indexes created and maintained via `sysand index`.
      (Added to §15: third-party tooling may serve a conformant tree
      but creation/maintenance via other means is not supported.)
      **Final check: confirmed resolved.**

---

## `docs/src/index-protocol.md`

- [x] L10 (J) — "Only clients?"
      [r3130122807](https://github.com/sensmetry/sysand/pull/288#discussion_r3130122807)
      **Final check: confirmed resolved.**
- [x] L17 (J) — RFC 2119 word list: `REQUIRED` is used but not listed,
      `SHOULD` is listed but not used. Suggest listing the whole set so
      the list doesn't need to be maintained as the doc evolves.
      [r3130163609](https://github.com/sensmetry/sysand/pull/288#discussion_r3130163609)
      **Final check: confirmed resolved.**
- [x] L27 (J) — Add a link to section 10 here (and in other places
      that refer to it). (Applied reference-style `[§N]` links
      throughout, with definitions at bottom of file.)
      [r3130137342](https://github.com/sensmetry/sysand/pull/288#discussion_r3130137342)
      **Final check: confirmed resolved.**
- [x] L61 (J) — Add links to the RFCs (or specific sections) that are
      referenced, e.g. https://www.rfc-editor.org/rfc/rfc3986.html#section-4.3.
      (All RFC references in the doc — RFC 2119, RFC 3986 §§2.3, 4.3,
      5.2.4, 6.2.2, RFC 3987 §5.3.2, RFC 5952 — are hyperlinked via
      reference-style `[rfcNNNN-xy]` defs consolidated at the bottom.)
      [r3130215814](https://github.com/sensmetry/sysand/pull/288#discussion_r3130215814)
      **Final check: confirmed resolved.**
- [x] L66 (J) — Prefer keeping the current restriction forever. OK to
      note that a future version _may_ relax it, but no strong reason
      to signal that it will. (Dropped the "future version MAY relax
      this" clause.)
      [r3130232961](https://github.com/sensmetry/sysand/pull/288#discussion_r3130232961)
      **Final check: confirmed resolved.**
- [x] L107 (J) — Clarify whether users are expected to write a full
      `pkg:sysand/<publisher>/<name>` when adding a dependency (e.g.
      `sysand add …`). (Resolved directly on the GitHub thread.)
      [r3130288390](https://github.com/sensmetry/sysand/pull/288#discussion_r3130288390)
      **Final check: confirmed resolved.**
- [x] L127 (J) — "which are equivalent to … plus the following …" is
      the wrong formulation; the function doc says "effectively
      equivalent to taking the following steps in order". Align the
      wording. (Aligned: §5 step 1 now reads "is effectively
      equivalent to taking the following steps in order, drawing on
      RFC 3986 §6.2.2…".)
      [r3130332044](https://github.com/sensmetry/sysand/pull/288#discussion_r3130332044)
      **Final check: confirmed resolved.**
- [x] L163 (J) — Clarify what "in total" means in this sentence.
      [r3130385266](https://github.com/sensmetry/sysand/pull/288#discussion_r3130385266)
      **Final check: confirmed resolved.**
- [x] L191 (J) — Apply suggestion: `A 200 response with
`{ "projects": [] }` is the "empty index" signal.`
      [r3130487251](https://github.com/sensmetry/sysand/pull/288#discussion_r3130487251)
      **Final check: confirmed resolved.**
- [x] L195 (J) — Contradiction: either clients surface this as a hard
      error, or the resolver chain skips this source; it can't be
      both. (Harmonized with §8 parallel: surface 404 as warning to
      user; resolver chain MAY skip. Also wrapped §8's
      `{ "versions": [] }` in braces to match §7's
      `{ "projects": [] }` style.)
      [r3130403783](https://github.com/sensmetry/sysand/pull/288#discussion_r3130403783)
      **Final check: confirmed resolved.**
- [x] L225 (J) — "Why not?" — justify the listed restriction or drop
      it. (Added rationale: semver build metadata has equal precedence,
      would break `(iri, version)` identity. Also noted via GitHub
      reply that prior art aligns: crates.io rejects build metadata on
      publish, PyPI forbids local-version identifiers on upload, npm
      discourages them due to the same equal-precedence collision, Go
      modules only special-case `+incompatible`.)
      [r3130506189](https://github.com/sensmetry/sysand/pull/288#discussion_r3130506189)
      **Final check: confirmed resolved (reply posted on GitHub).**
- [x] L307 (J) — Claim appears to be inaccurate; see
      `core/src/project/mod.rs:223`. (Added caveat: claim holds
      because §10 `meta.checksum` rule constrains `.meta.json` to
      SHA-256; without that, canonical form falls back to reading
      sources.)
      [r3130623372](https://github.com/sensmetry/sysand/pull/288#discussion_r3130623372)
      **Final check: confirmed resolved.**
- [x] L317 (J) — State explicitly that the `algorithm` specifier for
      each file must be `SHA256`. (Rewrote `meta.checksum` spec to
      show `{value, algorithm}` shape and require `algorithm:
"SHA256"` in v0. Also dropped the redundant "Other algorithms are
      reserved for future versions" trailing sentence in the
      `meta.checksum values` section.)
      [r3130644214](https://github.com/sensmetry/sysand/pull/288#discussion_r3130644214)
      **Final check: confirmed resolved.**
- [x] L333 (J) — Clarify whether a version directory can be removed
      later (e.g. a version that exposes sensitive data). Propose
      allowing removal. (Declined — immutability is intentionally
      absolute; lockfile reproducibility is held above incident-response
      ergonomics. Reply posted on the thread.)
      [r3130672092](https://github.com/sensmetry/sysand/pull/288#discussion_r3130672092)
      **Final check: confirmed resolved (decline posted on GitHub).**
- [x] L371 (J) — Specify a way to remove a version in v0 too, so a
      package author can scrub a version that accidentally exposed
      sensitive info. (Declined — immutability is intentionally
      absolute; lockfile reproducibility is held above incident-response
      ergonomics. Reply posted on the thread.)
      [r3130719984](https://github.com/sensmetry/sysand/pull/288#discussion_r3130719984)
      **Final check: confirmed resolved (decline posted on GitHub).**
- [x] L383 (J) — State that `sysand index` / `sysand client` apply
      less strict quality checks than `sysand server` / `sysand
publish`, and list the exact differences. (May be out of scope
      for this PR.) (Addressed by the §15 "wire-level vs
      semantic-project-quality" paragraph. Does not enumerate every
      rule — full quality-gate listing can be a follow-up. Reply
      posted on the thread.)
      [r3130190086](https://github.com/sensmetry/sysand/pull/288#discussion_r3130190086)
      **Final check: confirmed resolved.**

---

## `core/src/commands/lock.rs`

- [x] L178 (A) — Apply suggestion: `Some(info) => format!("`{}` {}",
info.name, info.version),`
      [r3128343526](https://github.com/sensmetry/sysand/pull/288#discussion_r3128343526)
      **Final check: confirmed resolved.**
- [x] L183 (A) — Apply suggestion: `return format!("`{iri}`");`
      [r3128346762](https://github.com/sensmetry/sysand/pull/288#discussion_r3128346762)
      **Final check: confirmed resolved.**
- [x] L186 (A) — Apply suggestion: `Ok(Some(name)) =>
format!("`{name}`"),`. Also: why no version? (Backtick formatting
      was applied earlier. For the "why no version?" follow-up we
      verified the branch was unreachable for every current
      `ProjectRead` impl — no concrete type overrides the default
      `name()` helper, which itself delegates to `get_info()`, and
      the branch is only reached when `get_info()` returned `None`.
      Rather than wire in a symmetric `project.version()` call for a
      dead case, applied YAGNI and deleted the `project.name()`
      branch; the closure now collapses to IRI-or-`<unknown>`.)
      [r3128350995](https://github.com/sensmetry/sysand/pull/288#discussion_r3128350995)
      **Final check: confirmed resolved.**
- [x] L315–321 (A) — "What index?" — error does not include indices,
      `lock_symbols` is local, and `lock` is not modified until after
      this check. Tighten the wording. (Collapsed the two-pass loop to
      a single pass and dropped the misleading "caller's collision
      table" comment — the two-pass was defensive against a failure
      mode that the early-`return Err` already precludes.)
      [r3128375344](https://github.com/sensmetry/sysand/pull/288#discussion_r3128375344)
      **Final check: confirmed resolved.**
- [x] L326–335 (A) — If this stays in, add a separate error case for
      when a project conflicts with itself. (Added
      `LockError::SelfNameCollision(SelfNameCollisionError { symbol,
project })` — single project in the error payload — distinct
      from the cross-project `NameCollision`.)
      [r3128378838](https://github.com/sensmetry/sysand/pull/288#discussion_r3128378838)
      **Final check: confirmed resolved.**

## `core/src/commands/publish.rs`

- [x] L256–260 (A) — Doc-comment for `PublishPreparation` is
      redundant; apply the compact suggestion.
      [r3128538550](https://github.com/sensmetry/sysand/pull/288#discussion_r3128538550)
      **Final check: confirmed resolved.**
- [x] L267–271 (A) — Tighten doc for `prepare_publish_payload` (does
      not touch network; should be called before any network activity).
      [r3128535104](https://github.com/sensmetry/sysand/pull/288#discussion_r3128535104)
      **Final check: confirmed resolved.**

## `sysand/src/commands/publish.rs`

- [x] L64 (A) — Use destructuring assignment.
      [r3128623595](https://github.com/sensmetry/sysand/pull/288#discussion_r3128623595)
      **Final check: confirmed resolved.**

## `core/src/env/discovery.rs`

- [ ] L4–17 (A) — Consider a simpler design like `uv`'s, where
      separate URLs are specified by the user for downloading vs.
      publishing directly. (Architectural question; left for
      discussion.)
      [r3128559401](https://github.com/sensmetry/sysand/pull/288#discussion_r3128559401)
- [x] L95–98 (A) — RFC 8615 mandates `discovery_root` to have an empty
      path, but we don't require/check this. (Moot: commit ed54557
      dropped `.well-known` in favor of `sysand-index-config.json`,
      which has no RFC 8615 constraint.)
      [r3128583481](https://github.com/sensmetry/sysand/pull/288#discussion_r3128583481)
      **Final check: confirmed resolved.**
- [x] L108 (A) — `.`-prefixed segments that aren't `/` are
      semantically the same as any random string; re-check the
      path-resolution reasoning. (Moot: applied to the old `.well-known`
      logic, dropped in ed54557.)
      [r3128612358](https://github.com/sensmetry/sysand/pull/288#discussion_r3128612358)
      **Final check: confirmed resolved.**
- [x] L179–180 (A) — `url::Url::path` already guarantees the leading
      `/`; rely on that instead of re-checking. (Rewrote
      `with_trailing_slash` to use `path_segments_mut`, eliminating the
      dead `is_empty()` branch.)
      [r3128649744](https://github.com/sensmetry/sysand/pull/288#discussion_r3128649744)
      **Final check: confirmed resolved.**
- [x] L181–184 (A) — Use the approach from `build_upload_url` and then
      re-use this function there. (`with_trailing_slash` is now the
      shared `path_segments_mut`-based helper; `build_upload_url` calls
      it.)
      [r3128668682](https://github.com/sensmetry/sysand/pull/288#discussion_r3128668682)
      **Final check: confirmed resolved.**
- [x] L193 (A) — "What concurrent callers?" `ResolvedEndpoints` should
      be created exactly once per index and then passed around.
      (Rewrote the doc to name the actual scenario: parallel solving
      can race the first async use of a sync-constructed env. Keeping
      the lazy `OnceCell` rather than making env construction async.)
      [r3128689660](https://github.com/sensmetry/sysand/pull/288#discussion_r3128689660)
      **Final check: confirmed resolved.**

## `core/src/env/discovery_tests.rs`

- [x] L20–21 (A) — Test passes even without `with_trailing_slash`;
      strengthen or drop. (Dropped
      `with_trailing_slash_sets_root_on_empty` — `url::Url::parse` of
      `https://example.com` already normalizes the path to `/`, so the
      test was tautological.)
      [r3128701843](https://github.com/sensmetry/sysand/pull/288#discussion_r3128701843)
      **Final check: confirmed resolved.**

## `core/src/env/iri_normalize.rs`

- [x] L26 (A) — Why not take `Iri` (instead of a string)? (Public
      signature changed to `fn normalize_iri_for_hash(iri: &Iri<&str>)`.
      Callers parse up front.)
      [r3128712709](https://github.com/sensmetry/sysand/pull/288#discussion_r3128712709)
      **Final check: confirmed resolved.**
- [ ] L34 (A) — Return `Iri<String>`, and make it clear that `s` is a
      full IRI. (Helpers take `&Iri<String>` internally; the outer
      pipeline threads the parsed form so the `s`-as-IRI intent is
      explicit in the type, not a doc-string caveat. **Pending a more
      careful look before final sign-off.**)
      [r3128752291](https://github.com/sensmetry/sysand/pull/288#discussion_r3128752291)
- [x] L35 (A) — Don't re-parse. If input is required to be an IRI,
      take it as an `Iri` arg.
      [r3128730807](https://github.com/sensmetry/sysand/pull/288#discussion_r3128730807)
      **Final check: confirmed resolved.**
- [ ] L48 (A) — Use `iri::builder()` instead. We'll need an `Iri`
      afterwards, so return `Iri`. (Declined: the builder's strict
      typestate requires reconstructing scheme/authority/path/query/
      fragment from scratch; for a "change only the host" rewrite it's
      more code than the known-valid splice. Needs a reply on the
      GitHub thread.)
      [r3128742685](https://github.com/sensmetry/sysand/pull/288#discussion_r3128742685)
- [x] L62–64 (A) — Re-parsing again; remove.
      [r3128872146](https://github.com/sensmetry/sysand/pull/288#discussion_r3128872146)
      **Final check: confirmed resolved.**
- [x] L66–67 (A) — Use `SCHEME_HTTP` / `SCHEME_HTTPS` constants
      instead of string literals.
      [r3129645934](https://github.com/sensmetry/sysand/pull/288#discussion_r3129645934)
      **Final check: confirmed resolved.**
- [ ] L80 (A) — "Again, why reference?" — take by value where
      appropriate. (Unclear which parameter: L80 currently sits on the
      `IriNormalizeError::Parse(ParseError)` variant. Needs a reply on
      the GitHub thread to clarify.)
      [r3129924205](https://github.com/sensmetry/sysand/pull/288#discussion_r3129924205)

## `core/src/lock.rs`

- [x] L184–196 (A) — Names are confusingly similar; be more specific.
      (Renamed `InvalidChecksumFormat` →
      `InvalidProjectDigestFormat`, `InvalidSourceChecksumFormat` →
      `InvalidRemoteKparDigestFormat`, with matching message rewordings.)
      [r3130693961](https://github.com/sensmetry/sysand/pull/288#discussion_r3130693961)
      **Final check: confirmed resolved.**
- [x] L361–386 (A) — Replace the roundabout construction with regular
      iteration. (Rewrote both validators as nested `for` loops with
      early-return on the first failure.)
      [r3130702960](https://github.com/sensmetry/sysand/pull/288#discussion_r3130702960)
      **Final check: confirmed resolved.**
- [x] L571–573 (A) — We are adding more to the lockfile anyway; do
      this change now instead of deferring. (`Source::Registry` →
      `Source::Index`, TOML key `registry` → `index`, `SOURCE_ENTRIES`
      updated, lock_tests fixture strings updated. Only one call site
      outside `lock.rs` touched this variant; TOML round-trip tests
      pass.)
      [r3130713057](https://github.com/sensmetry/sysand/pull/288#discussion_r3130713057)
      **Final check: confirmed resolved.**
- [ ] L580–581 (A) — Require (rather than optional) here. (Initial
      position was to decline: `None` correctly represents direct-HTTP
      kpars where no digest is advertised at source-discovery time,
      and the indexed path always populates it. **Reopening:** instead
      of one `RemoteKpar` with an optional digest, split the type —
      e.g. an `IndexKpar` (or similar) variant where the digest is
      required, distinct from the direct-HTTP case. Gets the
      type-level guarantee the reviewer wanted without breaking
      direct-HTTP deps or forcing a hash-at-lock download.)
      [r3130714642](https://github.com/sensmetry/sysand/pull/288#discussion_r3130714642)

## `core/src/project/any.rs`

- [ ] L99 (A) — Require `remote_kpar_digest` to be present.
      **Reopened** alongside `lock.rs` L580–581: pending the
      `IndexKpar` vs direct-HTTP split decision. See that item for the
      shared rationale.
      [r3129934403](https://github.com/sensmetry/sysand/pull/288#discussion_r3129934403)

## `core/src/project/utils.rs`

- [ ] L192–197 (A) — `fs::rename` does not work across
      mountpoints/drives. If there's any chance of that, use
      `local_directory::utils::move_fs_item()` instead. (Inlined the
      cross-device fallback into `wrapfs::rename`: tries `fs::rename`,
      falls back to copy + remove on `CrossesDevices`. Deleted
      `move_fs_item` and `copy_dir_recursive` from
      `env/local_directory/utils.rs`; its callers now use
      `wrapfs::rename` directly. The one existing external caller
      (`project/reqwest_kpar_download.rs:319`) picks up the safer
      semantics with no code change. **Pending more review.**)
      [r3129966336](https://github.com/sensmetry/sysand/pull/288#discussion_r3129966336)

## `core/src/purl.rs`

- [x] L32 (A) — Apply suggestion: `self == FieldKind::Name`.
      [r3130735526](https://github.com/sensmetry/sysand/pull/288#discussion_r3130735526)
      **Final check: confirmed resolved.**

## `core/src/resolve/combined.rs`

- [ ] L169 (A) — Failure here should indicate an I/O error — this
      isn't an index project. (Rewrote the comment: the remote
      resolver in this branch is the non-index network resolver, so a
      failure is an I/O / parse error on fetched project files, not an
      advertised-digest check. **Pending more review.**)
      [r3130086962](https://github.com/sensmetry/sysand/pull/288#discussion_r3130086962)
- [x] L298 (A) — `None` from `checksum_canonical_hex` always means
      `.project.json` / `.meta.json` don't exist. Document this on the
      trait, and restore the more precise previous message. (Trait
      doc on `ProjectRead::checksum_canonical_hex` now states the
      `Ok(None)` semantics; the log message is more precise.)
      [r3130148371](https://github.com/sensmetry/sysand/pull/288#discussion_r3130148371)
      **Final check: confirmed resolved.**

## `core/src/resolve/net_utils.rs`

- [x] L84–91 (A) — Reduce verbosity; if tests cover this, the
      narrative isn't needed. (Trimmed the redirect-policy doc on
      `create_reqwest_client` to the essential invariant; dropped the
      test-filename registry.)
      [r3130285469](https://github.com/sensmetry/sysand/pull/288#discussion_r3130285469)
      **Final check: confirmed resolved.**

## `core/src/resolve/priority_tests.rs`

- [x] L39 (A) — Change timestamp precision to seconds. (Subsumed by
      the broader `model::format_created` DRY: all producers route
      through one helper that emits `SecondsFormat::Secs`. No direct
      `to_rfc3339_opts` / `SecondsFormat` callers remain.)
      [r3130592514](https://github.com/sensmetry/sysand/pull/288#discussion_r3130592514)
      **Final check: confirmed resolved.**
