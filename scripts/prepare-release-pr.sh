#!/usr/bin/env bash
# Run only on the release-please branch, after its version updates.
set -euo pipefail

branch=$(git branch --show-current)
case "$branch" in
  release-please--branches--*) ;;
  *) echo 'Notice preparation requires a release-please branch.' >&2; exit 1 ;;
esac
if [[ -n $(git status --porcelain) ]]; then
  echo 'Notice preparation requires a clean checkout.' >&2
  exit 1
fi

python3 scripts/generate-license-notices.py
python3 scripts/generate-license-notices.py --check-inputs
git add -- THIRD_PARTY_NOTICES.txt
if ! git diff --cached --quiet -- THIRD_PARTY_NOTICES.txt; then
  git -c user.name='github-actions[bot]' \
    -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
    commit -m 'chore: regenerate release license notices' -- THIRD_PARTY_NOTICES.txt
  git push origin "HEAD:refs/heads/$branch"
fi
printf 'sha=%s\n' "$(git rev-parse HEAD)" >> "$GITHUB_OUTPUT"
