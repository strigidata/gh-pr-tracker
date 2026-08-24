# Render the open-PR board, split into the three things a developer does with it.
#
#   1. YOUR PRS         - you act. Ready-to-merge and conflicts first, drafts last.
#   2. NEED YOUR REVIEW - you review. Most-reviewable first.
#   3. OTHER DRAFTS     - completeness only.
#
# What counts as blocking is read from the repository's branch protection rule,
# not decided here. Only checks in requiredStatusCheckContexts can stop a merge;
# everything else runs and is ignored by the merge button. When no rule applies
# -- no protection, or a PR targeting another branch as in a stacked workflow --
# every failing check is treated as blocking, because silence in the policy
# means "we cannot tell", not "nothing matters".

def esc: "";
def R: esc + "[0m";
def BOLD: esc + "[1m";
def DIM: esc + "[2m";
def RED: esc + "[31m";
def GRN: esc + "[32m";
def YEL: esc + "[33m";
def BLU: esc + "[34m";
def MAG: esc + "[35m";

def age($t):
  (now - ($t | fromdateiso8601)) as $s
  | if   $s < 3600  then "\(($s/60)|floor)m"
    elif $s < 86400 then "\(($s/3600)|floor)h"
    else                 "\(($s/86400)|floor)d"
    end;

def vis: gsub("\\[[0-9;]*m"; "") | length;
def pad($s; $w): $s + (" " * (if ($w - ($s|vis)) > 0 then ($w - ($s|vis)) else 1 end));

(.data.viewer.login)                       as $me
| (.data.repository.defaultBranchRef.name) as $default
| ([ .data.repository.branchProtectionRules.nodes[]? | select(.pattern == $default) ] | first) as $rule
| (($rule.requiredStatusCheckContexts // []))       as $required
| (($rule.requiresStatusChecks == true) and (($required | length) > 0)) as $haveRequired
| (($rule.requiresConversationResolution // false)) as $convBlocks
| (($rule.requiredApprovingReviewCount // 0))       as $needApprovals

| .data.repository.pullRequests.nodes
| map(
    . as $p
    | ($p.commits.nodes[0].commit) as $hc
    | ([ $hc.statusCheckRollup.contexts.nodes[]?
         | if .__typename == "CheckRun"
           then { name: .name,    state: (.conclusion // .status // "PENDING") }
           else { name: .context, state: .state } end ]) as $checks
    # A rule governs only PRs targeting the branch it protects. A stacked PR
    # aimed at another feature branch is ungoverned, so nothing is advisory.
    | (($p.baseRefName == $default) and $haveRequired) as $governed
    | ([ $checks[] | select(.state as $s | ["FAILURE","ERROR","TIMED_OUT","STARTUP_FAILURE","ACTION_REQUIRED"] | index($s)) ]) as $failed
    | ([ $checks[] | select(.state as $s | ["IN_PROGRESS","QUEUED","PENDING","WAITING","EXPECTED"] | index($s)) ]) as $running
    | ([ $failed[] | select(($governed | not) or ((.name as $n | $required | index($n)) != null)) ]) as $failBlocking
    | ([ $p.reviewRequests.nodes[]?.requestedReviewer | (.login // .slug) | select(. != null) ]) as $wanted
    | { n: $p.number, author: ($p.author.login // "unknown"),
        draft: $p.isDraft, governed: $governed,
        failBlock: ($failBlocking | length),
        failAdv:   (($failed | length) - ($failBlocking | length)),
        run: ($running | length), tot: ($checks | length),
        ok: (($checks | length) - ($failed | length) - ($running | length)),
        threads:  ([ $p.reviewThreads.nodes[]? | select(.isResolved | not) ] | length),
        comments: ($p.comments.totalCount // 0),
        mergeable: $p.mergeable, msg: $p.mergeStateStatus, decision: $p.reviewDecision,
        wanted: $wanted,
        blockers: ([ $p.latestReviews.nodes[]? | select(.state == "CHANGES_REQUESTED") | .author.login ]),
        pushed: $hc.committedDate,
        mine: (($p.author.login // "") == $me),
        forme: ($wanted | index($me) != null) })
| map(
    . + {
      # Unresolved threads are red only where the repository enforces
      # resolution. Otherwise they are informational.
      conv: ((if .threads > 0 then (if $convBlocks then RED else YEL end) + "\(.threads)" + R
              else DIM + "0" + R end)
             + DIM + ".\(.comments)" + R),
      ci: (
        if   .draft         then DIM + "draft" + R
        elif .failBlock > 0 then RED + "x \(.failBlock)" + R + (if .failAdv > 0 then DIM + "+\(.failAdv)" + R else "" end)
        elif .failAdv > 0   then DIM + "! \(.failAdv) adv" + R
        elif .run  > 0      then YEL + "~ \(.run)/\(.tot) run" + R
        elif .tot == 0      then DIM + "no checks" + R
        else                     GRN + "ok \(.ok)/\(.tot)" + R
        end),
      mg: (
        if   .draft or .failBlock > 0 or .run > 0 then DIM + "-" + R
        elif .mergeable == "CONFLICTING"          then RED + "CONFLICT" + R
        elif .mergeable == "UNKNOWN"              then DIM + "computing" + R
        elif .msg == "BEHIND"                     then DIM + "behind" + R
        else                                           GRN + "clean" + R
        end),
      blocked: (
        if   .draft                       then DIM + "draft - not ready" + R
        elif (.blockers | length) > 0     then RED + "changes requested by \(.blockers | join(", "))" + R
        elif .failBlock > 0               then RED + "required check failing" + R
        elif $convBlocks and .threads > 0 then MAG + "\(.threads) unresolved thread(s)" + R
        elif .decision == "APPROVED" and .mergeable == "CONFLICTING" then RED + "approved - REBASE NEEDED" + R
        elif .decision == "APPROVED" and .run > 0 then YEL + "approved - CI running" + R
        elif .decision == "APPROVED"      then GRN + "READY TO MERGE" + R
        elif .run > 0                     then YEL + "CI running" + R
        elif (.wanted | length) > 3       then BLU + "review: \(.wanted[0:2] | join(", ")) +\((.wanted|length)-2)" + R
        elif (.wanted | length) > 0       then BLU + "review: \(.wanted | join(", "))" + R
        else                                   BLU + "NOBODY ASSIGNED" + R
        end),
      act: (
        if   .draft then 90
        elif .decision == "APPROVED" and .mergeable != "CONFLICTING" and .failBlock == 0 and .run == 0 then 0
        elif .mergeable == "CONFLICTING"  then 1
        elif (.blockers | length) > 0     then 2
        elif .failBlock > 0               then 3
        elif $convBlocks and .threads > 0 then 4
        elif .run > 0                     then 5
        else                                   6
        end),
      ready: (
        if   .failBlock > 0               then 50
        elif (.blockers | length) > 0     then 40
        elif .run  > 0                    then 30
        elif .tot == 0                    then 20
        elif .mergeable == "CONFLICTING"  then 12
        elif $convBlocks and .threads > 0 then 10
        else                                   0
        end) })
| . as $all
| ([ $all[] | select(.mine) ]                                           | sort_by(.act, .pushed))   as $mine
| ([ $all[] | select(.mine|not) | select(.forme) | select(.draft|not) ] | sort_by(.ready, .pushed)) as $review
| ([ $all[] | select(.mine|not) | select(.draft) ] | sort_by(.pushed) | reverse)                    as $odrafts
| ([ $all[] | select(.mine|not) | select(.forme|not) | select(.draft|not) ])                        as $else

| def row: (if .draft or .ready >= 30 then DIM else "" end)
         + pad("#\(.n)"; 7) + pad(.author[0:13]; 14) + pad(.ci; 15) + pad(.conv; 8)
         + pad(.mg; 11) + pad(.blocked; 40) + age(.pushed) + R;
  def head($t; $n): BOLD + $t + R + DIM + "  (\($n))" + R;
  def cols: DIM + pad("PR"; 7) + pad("AUTHOR"; 14) + pad("CI"; 15) + pad("OPEN.CMT"; 8)
                + pad("MERGE"; 11) + pad("WHY IT CANNOT MERGE"; 40) + "SINCE PUSH" + R;

  (DIM + "policy on \($default): "
       + (if $haveRequired then "\($required|length) required checks" else "NO required checks - every failure treated as blocking" end)
       + " | \($needApprovals) approval(s)"
       + (if $convBlocks then " | threads must be resolved" else "" end)
       + (if ($rule.dismissesStaleReviews // false) then " | a push dismisses approvals" else "" end)
       + R),
  "",
  head("YOUR PRS"; ($mine | length)), cols,
  (if ($mine|length) == 0 then DIM + "  nothing open" + R else ($mine[] | row) end),
  "",
  head("NEED YOUR REVIEW"; ($review | length)), cols,
  (if ($review|length) == 0 then DIM + "  nothing waiting on you" + R else ($review[] | row) end),
  "",
  head("OTHER DRAFTS"; ($odrafts | length)),
  (if ($odrafts|length) == 0 then DIM + "  none" + R else ($odrafts[] | row) end),
  "",
  (DIM + "\($all|length) open | \($else|length) not your review"
       + " | \([ $all[] | select(.blocked | test("READY TO MERGE")) ] | length) ready to merge"
       + " | \([ $all[] | select(.mergeable == "CONFLICTING") | select(.draft|not) ] | length) conflicting"
       + " | \([ $review[] | select(.ready < 30) ] | length) reviewable now" + R),
  (DIM + "CI: x=blocking, !=advisory (runs, cannot block), ~=running"
       + "   OPEN.CMT=unresolved threads.comments"
       + "   MERGE shown only once CI is green" + R)
