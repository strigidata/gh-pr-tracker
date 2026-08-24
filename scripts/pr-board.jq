# Render the open-PR board.
# Input: the GraphQL response from scripts/pr-board.sh
#
# The last column answers one question: "why can't this merge yet?"
# Everything else is supporting evidence for that answer.

def esc: "";
def R: esc + "[0m";
def BOLD: esc + "[1m";
def DIM: esc + "[2m";
def RED: esc + "[31m";
def GRN: esc + "[32m";
def YEL: esc + "[33m";
def BLU: esc + "[34m";
def MAG: esc + "[35m";

# Sum the counts for a set of check states.
def cnt($a; $states): [ $a[]? | select(.state as $s | $states | index($s)) | .count ] | add // 0;

def age($t):
  (now - ($t | fromdateiso8601)) as $s
  | if   $s < 3600  then "\(($s/60)|floor)m"
    elif $s < 86400 then "\(($s/3600)|floor)h"
    else                 "\(($s/86400)|floor)d"
    end;

# Visible width, ignoring ANSI sequences.
def vis: gsub("\\[[0-9;]*m"; "") | length;
def pad($s; $w): $s + (" " * (if ($w - ($s|vis)) > 0 then ($w - ($s|vis)) else 1 end));

.data.repository.pullRequests.nodes
| map(
    . as $p
    | ($p.commits.nodes[0].commit) as $hc
    | ($hc.statusCheckRollup.contexts.checkRunCountsByState // []) as $cr
    | ($hc.statusCheckRollup.contexts.statusContextCountsByState // []) as $sc
    | (cnt($cr; ["FAILURE","ACTION_REQUIRED","TIMED_OUT","STARTUP_FAILURE"]) + cnt($sc; ["FAILURE","ERROR"])) as $fail
    | (cnt($cr; ["IN_PROGRESS","QUEUED","PENDING","WAITING"]) + cnt($sc; ["PENDING","EXPECTED"]))            as $run
    | (cnt($cr; ["SUCCESS","SKIPPED","NEUTRAL"]) + cnt($sc; ["SUCCESS"]))                                    as $ok
    | {
        n:         $p.number,
        url:       $p.url,
        author:    ($p.author.login // "unknown"),
        draft:     $p.isDraft,
        fail:      $fail,
        run:       $run,
        ok:        $ok,
        tot:       ($fail + $run + $ok),
        threads:   ([ $p.reviewThreads.nodes[]? | select(.isResolved | not) ] | length),
        mergeable: $p.mergeable,
        msg:       $p.mergeStateStatus,
        decision:  $p.reviewDecision,
        wanted:    ([ $p.reviewRequests.nodes[]?.requestedReviewer | (.login // .slug) | select(. != null) ]),
        blockers:  ([ $p.latestReviews.nodes[]? | select(.state == "CHANGES_REQUESTED") | .author.login ]),
        pushed:    $hc.committedDate,
        title:     ($p.title[0:40])
      }
  )
| map(
    . + {
      ci: (
        if   .draft    then DIM + "draft" + R
        elif .fail > 0 then RED + "x \(.fail)/\(.tot)" + R + (if .run > 0 then DIM + " +\(.run)" + R else "" end)
        elif .run  > 0 then YEL + "~ \(.run)/\(.tot) running" + R
        elif .tot == 0 then DIM + "no checks" + R
        else                GRN + "ok \(.ok)/\(.tot)" + R
        end),
      # Merge state is only meaningful once CI is green. Showing "conflict" on a
      # red PR is noise: fix the build first.
      mg: (
        if   .draft or .fail > 0 or .run > 0 then DIM + "-" + R
        elif .mergeable == "CONFLICTING"     then RED + "CONFLICT" + R
        elif .mergeable == "UNKNOWN"         then DIM + "computing" + R
        elif .msg == "BEHIND"                then YEL + "behind" + R
        else                                      GRN + "clean" + R
        end),
      # The answer to "why can't this merge yet?", in precedence order.
      blocked: (
        if   .draft                    then DIM + "draft - not ready" + R
        elif (.blockers | length) > 0  then RED + "changes requested by \(.blockers | join(", "))" + R
        elif .fail > 0                 then RED + "CI failing - author" + R
        elif .decision == "APPROVED" and .mergeable == "CONFLICTING" then RED + "approved - REBASE NEEDED" + R
        elif .decision == "APPROVED" and .run > 0 then YEL + "approved - CI running" + R
        elif .decision == "APPROVED"   then GRN + "READY TO MERGE" + R
        elif .threads > 0              then MAG + "\(.threads) open thread(s)" + R
        elif .run > 0                  then YEL + "CI running" + R
        elif (.wanted | length) > 3    then BLU + "waiting for review: \(.wanted[0:2] | join(", ")) +\((.wanted|length)-2)" + R
        elif (.wanted | length) > 0    then BLU + "waiting for review: \(.wanted | join(", "))" + R
        else                                BLU + "waiting for review - NOBODY ASSIGNED" + R
        end)
    })
| sort_by(.draft, (if .decision == "APPROVED" then 0 else 1 end), .pushed)
| (BOLD + pad("PR"; 7) + pad("AUTHOR"; 14) + pad("CI"; 16) + pad("THR"; 5)
        + pad("MERGE"; 11) + pad("WHY IT CANNOT MERGE"; 42) + "SINCE PUSH" + R),
  (.[] |
     pad("#\(.n)"; 7) + pad(.author[0:13]; 14) + pad(.ci; 16) + pad("\(.threads)"; 5)
   + pad(.mg; 11) + pad(.blocked; 42) + age(.pushed)),
  "",
  (DIM + "\(length) open PRs   "
       + "\([ .[] | select(.blocked | test("READY TO MERGE")) ] | length) ready to merge   "
       + "\([ .[] | select(.blocked | test("waiting for review")) ] | length) waiting on a reviewer   "
       + "\([ .[] | select(.blocked | test("REBASE|CI failing|changes requested")) ] | length) waiting on the author"
       + R)
