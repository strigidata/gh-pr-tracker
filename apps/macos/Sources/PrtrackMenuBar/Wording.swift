import PrtrackFFI

// The wording below is kept in sync BY HAND with `reason_cell` in
// crates/prtrack-cli/src/render.rs (the CLI's sentences). That is a spike
// limitation: `ReasonKind` carries no payload, so these are the payload-free
// forms of each sentence. The switch has no `default:`, so a new rung fails to
// compile here.
func wording(for reason: ReasonKind) -> String {
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
