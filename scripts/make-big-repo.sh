#!/bin/sh
# Builds the big test repository: 100,000 commits, 200 branches and a merge
# about every 20 commits. Everything is generated, so two runs give the same
# repository.
#
# Usage: scripts/make-big-repo.sh [destination] [commit-count]
set -eu

dest=${1:-target/big-repo}
count=${2:-100000}

if [ -e "$dest" ]; then
    echo "error: $dest already exists. Remove it first." >&2
    exit 1
fi

# Keep the developer's Git config out of it.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 LC_ALL=C

git init -q -b main "$dest"

awk -v count="$count" '
function emit(ref, parent, merged, msg,    body) {
    n++
    when = 1500000000 + n * 600
    who = "Dev " (n % 20) " <dev" (n % 20) "@example.com> " when " +0000"
    body = "line " n "\n"
    print "commit " ref
    print "mark :" n
    print "author " who
    print "committer " who
    print "data " length(msg)
    print msg
    if (parent) print "from :" parent
    if (merged) print "merge :" merged
    print "M 100644 inline src/file" (n % 50) ".txt"
    print "data " length(body)
    print body
    return n
}
BEGIN {
    main = 0; side = 0; branches = 0; n = 0
    while (n < count) {
        step = n % 20
        if (n % 500 == 250) {
            # A branch that is never merged: one commit off main.
            branches++
            emit(sprintf("refs/heads/feature/%03d", branches), main, 0, "Start feature " branches)
        } else if (main && (step == 5 || step == 7 || step == 9)) {
            side = emit("refs/heads/side", side ? side : main, 0, "Side work " (n + 1))
        } else if (side && step == 19) {
            main = emit("refs/heads/main", main, side, "Merge side work into main")
            side = 0
        } else {
            main = emit("refs/heads/main", main, 0, "Change " (n + 1) " on main")
        }
        if (n % 10000 == 0) {
            print "reset refs/tags/v" (n / 10000) ".0"
            print "from :" main
            print ""
        }
    }
}' | git -C "$dest" fast-import --quiet

# "side" only existed to hold commits until their merge.
git -C "$dest" update-ref -d refs/heads/side
git -C "$dest" checkout -q -f main

echo "$dest: $(git -C "$dest" rev-list --all --count) commits," \
    "$(git -C "$dest" rev-list --all --merges --count) merges," \
    "$(git -C "$dest" for-each-ref refs/heads | wc -l | tr -d ' ') branches"
