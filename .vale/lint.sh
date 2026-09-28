#!/usr/bin/env bash
# Prose lint for Nomos docs, per docs/style/prose-spec.yaml.
#
# 1. Lints every tracked Markdown file with the Kennedy Vale style.
# 2. Runs the spec's acceptance tests: every paragraph in fixtures/reject.md
#    must raise at least one alert, and fixtures/accept.md must raise none.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

command -v vale >/dev/null || { echo "vale not found: https://vale.sh/docs/install" >&2; exit 127; }

mapfile -t docs < <(git ls-files '*.md' ':!:.vale/fixtures/*')
vale "${docs[@]}"

vale --no-exit --output=line .vale/fixtures/accept.md > /tmp/nomos-vale-accept.txt
if [[ -s /tmp/nomos-vale-accept.txt ]]; then
  echo "acceptance: target prose raised alerts:" >&2
  cat /tmp/nomos-vale-accept.txt >&2
  exit 1
fi

flagged=$(vale --no-exit --output=line .vale/fixtures/reject.md | cut -d: -f2 | sort -u)
missed=0
while IFS= read -r n; do
  grep -qx "$n" <<<"$flagged" || { echo "acceptance: reject.md line $n raised no alert" >&2; missed=1; }
done < <(grep -n -v -e '^#' -e '^$' .vale/fixtures/reject.md | cut -d: -f1)
exit "$missed"
