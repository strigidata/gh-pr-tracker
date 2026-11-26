import PrtrackFFI

// The wording below is kept in sync BY HAND with crates/prtrack-cli/src/render.rs:
// `reason_cell` (one sentence per rung), `rereview_cell` (one per verdict), and
// the rule in render.rs that chooses between them (its
// `matches!(placed.reason(), BlockedReason::ChangesRequested ..)` branch),
// which `wording(for:)` mirrors. That is a spike limitation: `ReasonKind` carries no payload, so the
// reasons are the payload-free forms of each sentence. Neither switch has a
// `default:`, so a new rung or verdict fails to compile here.

/// The reason column's sentence for `row`, chosen as the CLI chooses it: the
/// reason when it is a change request, otherwise the re-review sentence when
/// the viewer's say was pushed past, otherwise the reason.
func wording(for row: PopoverRow) -> String {
    if case .changesRequested = row.reason { return wording(for: row.reason) }
    if let verdict = row.rereview { return wording(for: verdict) }
    return wording(for: row.reason)
}

private func wording(for reason: ReasonKind) -> String {
    switch reason {
    case .draft: "draft - not ready"
    case .changesRequested: "changes requested"
    case .requiredCheckFailing: "required check failing"
    case .unresolvedThreads: "unresolved threads"
    case .approvedButConflicting: "approved - REBASE NEEDED"
    case .conflicting: "conflicting - rebase first"
    case .behindBase: "approved - UPDATE BRANCH"
    case .approvedCiRunning: "approved - CI running"
    case .readyToMerge: "READY TO MERGE"
    case .ciRunning: "CI running"
    case .mergeabilityNotKnown: "mergeability not known"
    case .waitingForReview: "waiting for review"
    case .nobodyAssigned: "NOBODY ASSIGNED"
    }
}

private func wording(for verdict: Verdict) -> String {
    switch verdict {
    case .changesRequested: "RE-REVIEW: pushed since you blocked it"
    case .approved: "RE-REVIEW: pushed since you approved"
    }
}

// The hints below have no twin in the CLI, which says what to do through its
// exit code and its error chain. No `default:`, so a case Rust adds to
// `FetchError` fails to compile here.

/// What to do about `error`, said above its message.
func failureHint(for error: FetchError) -> String {
    switch error {
    case .NoToken: "Set GH_TOKEN or GITHUB_TOKEN before launching."
    case .Unauthorized: "GitHub rejected the token."
    case .RateLimited: "GitHub's rate limit is exhausted; try again later."
    case .RepositoryUnavailable: "Check PRTRACK_REPO: it is not owner/name, or this token cannot see it."
    case .Network: "GitHub could not be reached; try again."
    case .Unreadable: "GitHub's answer could not be read."
    }
}
