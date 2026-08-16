# Changelog

## [0.1.1](https://github.com/MikeGrier/globazog-rs/compare/v0.1.0...v0.1.1) (2026-08-16)


### Bug Fixes

* always emit ContainerEnds, even on cancellation, so the 1:1 enter/end invariant holds (D-64) ([960c231](https://github.com/MikeGrier/globazog-rs/commit/960c2311b655ea9f0a04902754ebeadbc45d2b09))
* build root canonical keys from lossless code points + D-28 fold, not to_string_lossy, so unpaired surrogates never merge distinct roots ([17700d9](https://github.com/MikeGrier/globazog-rs/commit/17700d99dd6fadd7cb645674723bd7bf9e6bf0ba))
* carry the root index on CqError so a multi-root query can attribute a root-open failure to a specific root ([64f26a0](https://github.com/MikeGrier/globazog-rs/commit/64f26a001b5dbfbc895aa836c0115e6c4b6be9c9))
* collapse consecutive ** segments per D-24 so **/**/x reduces to **/x ([be28a2f](https://github.com/MikeGrier/globazog-rs/commit/be28a2fe4327bdb7343ba9fc9271e781588f39c1))
* de-template dependabot.yml (drop the leftover conditional npm block for the deleted extension) so it is valid YAML again ([d8d2954](https://github.com/MikeGrier/globazog-rs/commit/d8d2954fd20b5399591aa84fb3ecde7de184137a))
* dedup supplied roots (D-37), reject zero ring_capacity, and require an absolute base for leading-separator patterns (D-32) ([48bad57](https://github.com/MikeGrier/globazog-rs/commit/48bad57e94cfa4e30ac2f8ebfe436b5f8cb63d35))
* emit the fatal error immediately before Terminal{Failed} (D-71 ordering) ([89b277b](https://github.com/MikeGrier/globazog-rs/commit/89b277b1747108637fda071c103f63ac2ef8491d))
* expose lowered roots on QueryHandle so Root(index) is resolvable for full-path reconstruction (incl. self-rooting patterns) ([c105b4d](https://github.com/MikeGrier/globazog-rs/commit/c105b4d18caae077082e685214f4f8afef6ef953))
* make Signal::wait_timeout spurious-wakeup-safe via wait_timeout_while predicate loop ([6fb4908](https://github.com/MikeGrier/globazog-rs/commit/6fb4908f323f5c78bc0ef3d827454d2b93147c6c))
* only schedule roots some pattern applies to, so an unrelated bad root can't fail an anchored traversal (D-38) ([e4d0037](https://github.com/MikeGrier/globazog-rs/commit/e4d0037e83f4180ec0825b475a2cc76eb6e7ebd2))
* open the directory before emitting ContainerEnter so an unopenable dir yields no phantom container (D-64) ([44ab3d0](https://github.com/MikeGrier/globazog-rs/commit/44ab3d0363f7a0a0b1ead1a47a424e212ce5601f))
* preserve collected entries on a late directory read error (DirScan contract, D-53) ([d79c8aa](https://github.com/MikeGrier/globazog-rs/commit/d79c8aac43821afb38cbe71085b584131602b814))
* preserve the failing entry name in EntryError so per-entry failures identify which entry (D-53) ([8fee26e](https://github.com/MikeGrier/globazog-rs/commit/8fee26ee4c2efc8b0466aa930ed9c396ad07933c))
* query Windows FileIdInfo only when file identity is requested (D-62) ([5aa4af0](https://github.com/MikeGrier/globazog-rs/commit/5aa4af089a86c76fc23532a19ad42f2ddecb87b1))
* reject a stray unescaped closing brace per D-45 instead of accepting it as a literal ([15d333a](https://github.com/MikeGrier/globazog-rs/commit/15d333ac14e61e2f1e93b1292cf03b6937091e0f))
* reject drive-relative paths (D-33), saturate portable timestamps, reject adjacent stars in alternation arms ([b298677](https://github.com/MikeGrier/globazog-rs/commit/b298677f21fb3c9e9e88e502e9f71be66519ee56))
* reject non-absolute roots (D-74) so worker-thread opens never race the process CWD; update docs/tests to resolve CWD at the caller edge ([9a67dc6](https://github.com/MikeGrier/globazog-rs/commit/9a67dc661dba9479d21012d5b6e4574c4dd09c1f))
* reject win drive-relative root without base (D-32); keep final literal segment when peeling (D-37) ([bc16c31](https://github.com/MikeGrier/globazog-rs/commit/bc16c31b03db56c6a9a36104b0a1b9525573d982))
* remove the inert SubmitQuery SQ op (sync engine boots via submit; reactor boot op tracked in M7-6) ([eb5cbe5](https://github.com/MikeGrier/globazog-rs/commit/eb5cbe5e299f1fe754ada6993b6d61d7cbbe8a8e))
* resolve dialect versions as a strict major.minor.patch triplet and reject overlong specs like posix@1.0.0.0 ([07e77b3](https://github.com/MikeGrier/globazog-rs/commit/07e77b3de207e466cacd3a7265b01ac16a1a43bf))
* resolve the glob example's dir argument against a CWD snapshot so the default '.' no longer panics under D-74 ([a41c3cf](https://github.com/MikeGrier/globazog-rs/commit/a41c3cffcaedb17e7d8414df30357d8c0c37ef91))
* route mandatory CQ pushes (ContainerEnd/fatal/terminal) through a cancel-immune timed recheck loop so concurrent workers cannot deadlock on a full ring ([cad21a6](https://github.com/MikeGrier/globazog-rs/commit/cad21a610e17bbea61806865dfaaab3deeb9c983))
* scope pattern matching to each pattern's applicable roots so anchored patterns don't cross-apply (D-38) ([20a7d3c](https://github.com/MikeGrier/globazog-rs/commit/20a7d3cdfc7fc2d7da2b69405d162b289502fe0b))
* split EnumPlan file-id into all-entries (want_file_id) vs reparse-only (want_reparse_file_id) so every backend honors identity requests consistently ([9dc37fa](https://github.com/MikeGrier/globazog-rs/commit/9dc37fa33f21b420f286beefd0e928ad86af379e))
* treat an unsupported FileIdInfo query as identity-unavailable (unknown id, pass-through) instead of failing the whole enumeration ([7fe92f9](https://github.com/MikeGrier/globazog-rs/commit/7fe92f9dd897a7b5294f761d2010eb0ed5913468))
* use the unknown FileId sentinel when statx omits STATX_INO, so distinct files on the same fs do not collide in cycle detection ([774c082](https://github.com/MikeGrier/globazog-rs/commit/774c0823270988fbc5486f3c1dedddefd8a254ad))
* wake-propagate in wait_nonempty so a coalesced signal cannot strand a second non-emptiness waiter ([3f2ff49](https://github.com/MikeGrier/globazog-rs/commit/3f2ff49ab0721cf285c5f01da3aa845a8e291c44))
* wake-propagation in CompletionRing::pop keeps the blocking wait_pop path correct under multiple consumers ([d7b3685](https://github.com/MikeGrier/globazog-rs/commit/d7b368566ef9b1affb2037725b495a56f4df3e88))


### Performance Improvements

* honor the fetch mask in enumeration so Linux skips statx when only names are needed (D-62) ([d042d17](https://github.com/MikeGrier/globazog-rs/commit/d042d17bd8a75e81eb42a9eac1ab72b0905bc332))
* reuse the per-directory path vector across entries instead of cloning it per entry ([8ad4945](https://github.com/MikeGrier/globazog-rs/commit/8ad49451927ecce8010e874ccb1ffa71847b5295))
