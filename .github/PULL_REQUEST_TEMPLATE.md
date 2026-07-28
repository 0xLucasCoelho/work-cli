## Summary

<!-- What does this PR change, and why? -->

## Scope check

`work` is a session + isolation manager (it deliberately does not install tools
or manage credentials). This change:

- [ ] fits that scope
- [ ] does not introduce secrets handling

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] (if behavior changed) `work doctor` passes and the relevant end-to-end path was exercised

## Notes

<!-- Anything reviewers should know, including leftover test workspaces to clean up. -->
