# Render the open-PR board, split into the three things a developer does with it.
# Input: the GraphQL response from scripts/pr-board.sh
#
#   1. YOUR PRS        — you act. Ready-to-merge and conflicts first, drafts last.
#   2. NEED YOUR REVIEW — you review. Most-reviewable first.
#   3. OTHER DRAFTS     — completeness only.
#
# The rightmost column always answers one question: why can't this merge yet?

def esc: "";
def R: esc + "[0m";
def BOLD: esc + "[1m";
def DIM: esc + "[2m";
def RED: esc + "[31m";
def GRN: esc + "[32m";
def YEL: esc + "[33m";
def BLU: esc + "[34m";
def MAG: esc + "[35m";

def cnt($a; $states): [ $a[]? | select(.state as $s | $states | index($s)) | .count ] | add // 0;

def age($t):
  (now - ($t | fromdateiso8601)) as $s
  | if   $s < 3600  then "\(($s/60)|floor)m"
    elif $s < 86400 then "\(($s/3600)|floor)h"
    else                 "\(($s/86400)|floor)d"
    end;

def vis: gsub("\u001b\\[[0-9;]*m"; "") | length;
def pad($s; $w): $s + (" " * (if ($w - ($s|vis)) > 0 then ($w - ($s|vis)) else 1 end));

(.data.viewer.login) as $me
| .data.repository.pullRequests.nodes
| map(
    . as $p
    | ($p.commits.nodes[0].commit) as $hc
    | ($hc.statusCheckRollup.contexts.checkRunCountsByState // []) as $cr
    | ($hc.statusCheckRollup.contexts.statusContextCountsByState // []) as $sc
    | (cnt($cr; ["FAILURE","ACTION_REQUIRED","TIMED_OUT","STARTUP_FAILURE"]) + cnt($sc; ["FAILURE","ERROR"])) as $fail
    | (cnt($cr; ["IN_PROGRESS","QUEUED","PENDING","WAITING"]) + cnt($sc; ["PENDING","EXPECTED"]))            as $run
    | (cnt($cr; ["SUCCESS","SKIPPED","NEUTRAL"]) + cnt($sc; ["SUCCESS"]))                                    as $ok
    | ([ $p.reviewRequests.nodes[]?.requestedReviewer | (.login // .slug) | select(. != null) ]) as $wanted
    | {
        n: $p.number, url: $p.url, author: ($p.author.login // "unknown"), draft: $p.isDraft,
        fail: $fail, run: $run, ok: $ok, tot: ($fail + $run + $ok),
        threads:   ([ $p.reviewThreads.nodes[]? | select(.isResolved | not) ] | length),
        mergeable: $p.mergeable, msg: $p.mergeStateStatus, decision: $p.reviewDecision,
        wanted: $wanted,
        blockers: ([ $p.latestReviews.nodes[]? | select(.state == "CHANGES_REQUESTED") | .author.login ]),
        pushed: $hc.committedDate,
        mine: (($p.author.login // "") == $me),
        forme: ($wanted | index($me) != null)
      })
| map(
    . + {
      ci: (
        if   .draft    then DIM + "draft" + R
        elif .fail > 0 then RED + "x \(.fail)/\(.tot)" + R + (if .run > 0 then DIM + " +\(.run)" + R else "" end)
        elif .run  > 0 then YEL + "~ \(.run)/\(.tot) run" + R
        elif .tot == 0 then DIM + "no checks" + R
        else                GRN + "ok \(.ok)/\(.tot)" + R
        end),
      # Merge state is meaningful only once CI is green. "conflict" on a red PR
      # is noise; fix the build first.
      mg: (
        if   .draft or .fail > 0 or .run > 0 then DIM + "-" + R
        elif .mergeable == "CONFLICTING"     then RED + "CONFLICT" + R
        elif .mergeable == "UNKNOWN"         then DIM + "computing" + R
        elif .msg == "BEHIND"                then YEL + "behind" + R
        else                                      GRN + "clean" + R
        end),
      blocked: (
        if   .draft                    then DIM + "draft - not ready" + R
        elif (.blockers | length) > 0  then RED + "changes requested by \(.blockers | join(", "))" + R
        elif .fail > 0                 then RED + "CI failing - author" + R
        elif .decision == "APPROVED" and .mergeable == "CONFLICTING" then RED + "approved - REBASE NEEDED" + R
        elif .decision == "APPROVED" and .run > 0 then YEL + "approved - CI running" + R
        elif .decision == "APPROVED"   then GRN + "READY TO MERGE" + R
        elif .threads > 0              then MAG + "\(.threads) open thread(s)" + R
        elif .run > 0                  then YEL + "CI running" + R
        elif (.wanted | length) > 3    then BLU + "review: \(.wanted[0:2] | join(", ")) +\((.wanted|length)-2)" + R
        elif (.wanted | length) > 0    then BLU + "review: \(.wanted | join(", "))" + R
        else                                BLU + "NOBODY ASSIGNED" + R
        end),
      # Urgency for your own PRs: what should you do first?
      act: (
        if   .draft then 90
        elif .decision == "APPROVED" and .mergeable != "CONFLICTING" and .fail == 0 and .run == 0 then 0
        elif .mergeable == "CONFLICTING" then 1
        elif (.blockers | length) > 0    then 2
        elif .fail > 0                   then 3
        elif .threads > 0                then 4
        elif .run > 0                    then 5
        else                                  6
        end),
      # Readiness for review: how worth looking at is this right now?
      ready: (
        if   .fail > 0                   then 50
        elif (.blockers | length) > 0    then 40
        elif .run  > 0                   then 30
        elif .tot == 0                   then 20
        elif .mergeable == "CONFLICTING" then 12
        elif .msg == "BEHIND"            then 11
        elif .threads > 0                then 10
        else                                  0
        end)
    })
| . as $all
| ([ $all[] | select(.mine) ]                                    | sort_by(.act, .pushed))   as $mine
| ([ $all[] | select(.mine | not) | select(.forme) | select(.draft | not) ] | sort_by(.ready, .pushed)) as $review
| ([ $all[] | select(.mine | not) | select(.draft) ]             | sort_by(.pushed) | reverse) as $odrafts
| ([ $all[] | select(.mine | not) | select(.forme | not) | select(.draft | not) ]) as $else

| def row: (if .draft or .ready >= 30 then DIM else "" end) + pad("#\(.n)"; 7) + pad(.author[0:13]; 14) + pad(.ci; 15) + pad("\(.threads)"; 4)
         + pad(.mg; 11) + pad(.blocked; 40) + age(.pushed) + R;
  def head($t; $n): BOLD + $t + R + DIM + "  (\($n))" + R;
  def cols: DIM + pad("PR"; 7) + pad("AUTHOR"; 14) + pad("CI"; 15) + pad("THR"; 4)
                + pad("MERGE"; 11) + pad("WHY IT CANNOT MERGE"; 40) + "SINCE PUSH" + R;

  head("YOUR PRS"; ($mine | length)),
  cols,
  (if ($mine | length) == 0 then DIM + "  nothing open" + R else ($mine[] | row) end),
  "",
  head("NEED YOUR REVIEW"; ($review | length)),
  cols,
  (if ($review | length) == 0 then DIM + "  nothing waiting on you" + R else ($review[] | row) end),
  "",
  head("OTHER DRAFTS"; ($odrafts | length)),
  (if ($odrafts | length) == 0 then DIM + "  none" + R else ($odrafts[] | row) end),
  "",
  (DIM + "\($all|length) open  ·  \($else|length) others' PRs you are not a reviewer on"
       + "  ·  \([ $all[] | select(.blocked | test("READY TO MERGE")) ] | length) ready to merge"
       + "  ·  \([ $all[] | select(.mergeable == "CONFLICTING") | select(.draft|not) ] | length) conflict with main (\([ $all[] | select(.mergeable == "CONFLICTING") | select(.draft|not) | select(.fail > 0) ] | length) hidden behind red CI)" + "  ·  \([ $review[] | select(.ready < 30) ] | length) reviewable now" + R)
