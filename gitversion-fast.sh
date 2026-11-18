#!/usr/bin/env bash
# gitversion-fast.sh
# Fast, dependency-free (git only) alternative to dotnet-gitversion.
# Outputs the same shape of JSON as GitVersion's FullSemVer/ShortSha for
# mainline-style versioning. NOT a full GitVersion re-implementation:
# it does not read GitVersion.yml, does not walk every branch, and uses
# `git describe` instead of full merge-base graph analysis. That's the
# whole point — it trades GitVersion's exhaustive branch walk (the thing
# that makes it slow) for a cheap, deterministic approximation.
#
# Usage:
#   bash gitversion-fast.sh            # from Linux/macOS/Docker
#   bash gitversion-fast.sh            # from Windows, via Git Bash's bash.exe
#                                       # (already on your machine if git is)
#
# Assumes annotated/lightweight tags like 1.2.3 or v1.2.3 on your main/release branch.

set -euo pipefail

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Error: not inside a git repository" >&2
  exit 1
fi

SHA=$(git rev-parse HEAD)
SHORT_SHA=$(git rev-parse --short=7 HEAD)

# Prefer CI-provided branch name (detached HEAD in CI checkouts otherwise),
# fall back to local branch, fall back to "detached".
BRANCH="${GITHUB_HEAD_REF:-${GITHUB_REF_NAME:-${BUILD_SOURCEBRANCHNAME:-}}}"
if [ -z "$BRANCH" ]; then
  BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "detached")
fi
if [ "$BRANCH" = "HEAD" ]; then
  BRANCH="detached"
fi

# Nearest reachable tag matching X.Y.Z or vX.Y.Z, plus commits since it.
DESCRIBE=$(git describe --tags --long --match '[0-9]*.[0-9]*.[0-9]*' --match 'v[0-9]*.[0-9]*.[0-9]*' 2>/dev/null || echo "")

if [ -z "$DESCRIBE" ]; then
  MAJOR=0; MINOR=1; PATCH=0
  HEIGHT=$(git rev-list --count HEAD)
else
  # DESCRIBE looks like: 2.6.278-4-gf7d3da5 or v2.6.278-4-gf7d3da5
  HEIGHT=$(echo "$DESCRIBE" | sed -E 's/^.*-([0-9]+)-g[0-9a-f]+$/\1/')
  TAG=$(echo "$DESCRIBE" | sed -E 's/^(.*)-[0-9]+-g[0-9a-f]+$/\1/')
  VERSION="${TAG#v}"
  MAJOR=$(echo "$VERSION" | cut -d. -f1)
  MINOR=$(echo "$VERSION" | cut -d. -f2)
  PATCH=$(echo "$VERSION" | cut -d. -f3)
fi

sanitize_branch() {
  echo "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//'
}
BRANCH_SLUG=$(sanitize_branch "$BRANCH")

if [ "$HEIGHT" -eq 0 ] && { [ "$BRANCH" = "main" ] || [ "$BRANCH" = "master" ]; }; then
  # Exactly on a release tag: clean release version.
  FULL_SEMVER="${MAJOR}.${MINOR}.${PATCH}"
elif [ "$BRANCH" = "main" ] || [ "$BRANCH" = "master" ]; then
  PATCH=$((PATCH + 1))
  FULL_SEMVER="${MAJOR}.${MINOR}.${PATCH}-alpha.1+${HEIGHT}"
else
  PATCH=$((PATCH + 1))
  FULL_SEMVER="${MAJOR}.${MINOR}.${PATCH}-${BRANCH_SLUG}.1+${HEIGHT}"
fi

cat <<EOF
{
  "FullSemVer": "${FULL_SEMVER}",
  "ShortSha": "${SHORT_SHA}",
  "Sha": "${SHA}",
  "BranchName": "${BRANCH}",
  "Major": ${MAJOR},
  "Minor": ${MINOR},
  "Patch": ${PATCH},
  "CommitsSinceTag": ${HEIGHT}
}
EOF
