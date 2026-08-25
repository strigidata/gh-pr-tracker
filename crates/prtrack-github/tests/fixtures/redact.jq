# Replace the user data in a recorded GitHub response with synthetic values.
#
# Input:  N recorded responses, slurped (`jq -s -f redact.jq page-1 page-2`).
# Output: an array of N redacted responses, in the order they were given.
#
# Every document is redacted against ONE mapping built from all of them, which
# is what makes a login redact to the same synthetic login on page two as on
# page one. Redacting the pages separately would renumber them independently
# and quietly destroy every relational assertion -- author versus reviewer
# versus viewer -- that the fixtures exist to support.
#
# The mapping is deterministic: distinct values are sorted and numbered, so the
# same recording redacts to the same bytes however many times it is run.
#
# What changes: logins, team slugs, pull request titles, branch names, the
# owner/name inside a URL, and -- under $redact_check_names -- check names.
# Nothing else.
#
# Check names are the join key of the whole policy decision: a required context
# is matched against a reported name literally, so both sides have to map
# through the same table or the blocking-versus-advisory split stops meaning
# anything. They do, which is why redacting them is safe. Measured on a real
# recording, the per-pull-request join structure -- attempts, distinct names,
# required-check matches, duplicate groups -- is identical before and after. In particular the structure is
# untouched -- no key is added or removed, no array changes length, and null
# stays distinct from absent -- because these fixtures are worth having only
# for being shaped exactly like a real response.
#
# What deliberately does not change:
#
#   enum values      the vocabulary under test.
#   timestamps       what orders one attempt against another.
#   counts, numbers  including pull request numbers.
#   cursors          what pagination is asserted on.
#   headRefOid       an opaque object id. It identifies nothing on its own to
#                    anyone who cannot already read the repository, and the
#                    pull request numbers beside it are kept for the same
#                    reason.

# The synthetic names. Deliberately unmistakable, so nobody reads a fixture and
# believes they are looking at a real account.
def synthetic_owner: "example-org";
def synthetic_repo: "example-repo";

# Branch names that name no project and give nothing away. Keeping them makes
# the fixture legible -- a pull request based on the default branch still
# visibly is -- and keeps the protection rule's pattern matching the branch it
# protects.
def public_branches: ["main", "master", "develop", "trunk"];

# `index_of(sorted; value)` numbered from one, as a zero-padded string.
def numbered($sorted; $value):
  (($sorted | index($value)) + 1)
  | tostring
  | if length < 2 then "0" + . else . end;

# Every string at a path this program rewrites, gathered across all documents.
def collect(f): [.[] | f] | flatten | map(select(type == "string")) | unique;

# A login can appear as the viewer, as an author, or as a requested reviewer.
# Gathered structurally rather than by path, so a login that turns up somewhere
# new after a query change is redacted rather than missed.
def all_logins: collect(.. | objects | select(has("login")) | .login);
def all_slugs: collect(.. | objects | select(has("slug")) | .slug);

def all_titles: collect(.data.repository.pullRequests.nodes[]?.title);

def all_branches:
  collect(
    (.data.repository.pullRequests.nodes[]?.baseRefName),
    (.data.repository.defaultBranchRef?.name),
    (.data.repository.branchProtectionRules.nodes[]?.pattern)
  )
  | map(select(. as $branch | public_branches | index($branch) | not));

def all_check_names:
  collect(
    (.data.repository.pullRequests.nodes[]?.commits.nodes[]?.commit.statusCheckRollup?.contexts.nodes[]?
      | (.name, .context)),
    (.data.repository.branchProtectionRules.nodes[]?.requiredStatusCheckContexts[]?)
  );

# The owner and the repository name, taken from the caller rather than guessed,
# so a repository whose name is a substring of something else cannot confuse it.
$owner as $owner
| $name as $name
| $redact_check_names as $redact_check_names
| . as $documents
| (all_logins) as $logins
| (all_slugs) as $slugs
| (all_titles) as $titles
| (all_branches) as $branches
| (all_check_names) as $check_names

# Each of these rewrites the string it is applied to, so they compose with
# `|=` at the paths below.
| def login: "user-" + numbered($logins; .);
  def slug: "team-" + numbered($slugs; .);
  def title: "Pull request title " + numbered($titles; .);
  def branch:
    . as $value
    | if public_branches | index($value) then $value
      else "topic/branch-" + numbered($branches; $value)
      end;
  def check:
    . as $value
    | if $redact_check_names == "yes" then "check-" + numbered($check_names; $value)
      else $value
      end;

  # A URL is rebuilt from its parts rather than pattern-matched, so an owner or
  # a repository name containing regex punctuation cannot slip through.
  def url:
    . as $value
    | ($value | split("/")) as $parts
    | if ($parts | length) > 4 and $parts[2] == "github.com"
      then $parts
        | (if .[3] == $owner then .[3] = synthetic_owner else . end)
        | (if .[4] == $name then .[4] = synthetic_repo else . end)
        | join("/")
      else $value
      end;

  def redact_actor: if type == "object" and has("login") then .login |= login else . end;

  def redact_context:
    if has("name") then .name |= check else . end
    | if has("context") then .context |= check else . end;

  def redact_pull_request:
    .title |= title
    | .url |= url
    | .baseRefName |= branch
    | .author |= redact_actor
    | (.reviewRequests.nodes[]?.requestedReviewer) |=
        (if type == "object" then
           (if has("login") then .login |= login else . end)
           | (if has("slug") then .slug |= slug else . end)
         else . end)
    | (.latestReviews.nodes[]?.author) |= redact_actor
    | (.commits.nodes[]?.commit.statusCheckRollup?.contexts.nodes[]?) |= redact_context;

  $documents
  | map(
      (.data.viewer) |= redact_actor
      | (.data.repository.defaultBranchRef?.name) |= branch
      | (.data.repository.branchProtectionRules.nodes[]?.pattern) |= branch
      | (.data.repository.branchProtectionRules.nodes[]?.requiredStatusCheckContexts[]?) |= check
      | (.data.repository.pullRequests.nodes[]?) |= redact_pull_request
    )
