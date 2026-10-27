// Exact trusted bootstrap; literal run block in gpu-vast.yml.
export const runnerBootstrap =
  [
    'set +x',
    'set -euo pipefail',
    '[[ "$GITHUB_REF" = refs/heads/master ]]',
    '[[ "$GITHUB_SHA" =~ ^[0-9a-f]{40}$ ]]',
    '[[ "$GITHUB_REPOSITORY" = reindertpelsma/kayfabe ]]',
    'temporary=$(mktemp -d)',
    'trap \'find "$temporary" -depth -delete\' EXIT',
    'for file in runner.py deployment.json; do',
    "  curl --silent --fail --proto '=https' --max-time 20 --max-filesize 1" + '31072 \\',
    '    -o "$temporary/$file" \\',
    '    "https://raw.githubusercontent.com/reindertpelsma/kayfabe/$GITHUB_' +
      'SHA/ci/vast-broker/$file" 2>/dev/null',
    'done',
    'env -i PATH=/usr/bin:/bin HOME="$temporary" \\',
    '  WORKER_URL="$WORKER_URL" WORKER_PATH="${WORKER_PATH:-}" \\',
    '  CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \\',
    '  CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \\',
    '  GITHUB_OUTPUT="$GITHUB_OUTPUT" GITHUB_STEP_SUMMARY="$GITHUB_STEP_SUM' + 'MARY" \\',
    '  GITHUB_EVENT_PATH="$GITHUB_EVENT_PATH" GITHUB_SHA="$GITHUB_SHA" \\',
    '  GITHUB_EVENT_NAME="$GITHUB_EVENT_NAME" GITHUB_REF="$GITHUB_REF" \\',
    '  GITHUB_REPOSITORY="$GITHUB_REPOSITORY" GITHUB_RUN_ID="$GITHUB_RUN_ID' + '" \\',
    '  GITHUB_RUN_ATTEMPT="$GITHUB_RUN_ATTEMPT" \\',
    '  ACTIONS_ID_TOKEN_REQUEST_TOKEN="$ACTIONS_ID_TOKEN_REQUEST_TOKEN" \\',
    '  ACTIONS_ID_TOKEN_REQUEST_URL="$ACTIONS_ID_TOKEN_REQUEST_URL" \\',
    '  python3 -I "$temporary/runner.py" "$temporary/deployment.json"',
  ].join('\n') + '\n';

export const sweepBootstrap = runnerBootstrap.replace(
  '"$temporary/deployment.json"\n',
  '"$temporary/deployment.json" --profile sweep\n',
);
