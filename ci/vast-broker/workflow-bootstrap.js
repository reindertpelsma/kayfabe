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
    // 2026-11-05: where runner.py may write the sanitised full log (fixed name inside; see below).
    '  KF_RUNLOG_DIR="$RUNNER_TEMP/gpu-vast-runlog" \\',
    '  python3 -I "$temporary/runner.py" "$temporary/deployment.json"',
  ].join('\n') + '\n';

// The sweep workflow has no artifact step, so it passes no log directory.
export const sweepBootstrap = runnerBootstrap
  .replace('  KF_RUNLOG_DIR="$RUNNER_TEMP/gpu-vast-runlog" \\\n', '')
  .replace('"$temporary/deployment.json"\n', '"$temporary/deployment.json" --profile sweep\n');

// The only artifact action the GPU workflow may hold, exact text (workflow-guard.js compares it).
export const runlogUploadStep = [
  '      - name: Upload sanitised full box log (untrusted text)',
  "        # 2026-11-05. The file is the full box log after the runner's sanitiser (runner.py",
  '        # write_runlog: every line `[untrusted] `, at most 4 MiB). This step has no env and no',
  '        # secret; it reads one fixed path, decides nothing, and a failure of it fails nothing.',
  "        # actions/upload-artifact@v4 uses the runner's results service, not GITHUB_TOKEN, so the job",
  '        # keeps `id-token: write` and gains no other permission (INFERRED, ci/vast-broker/README.md, runner log artifact).',
  "        if: ${{ always() && needs.pending.outputs.reused != 'true' }}",
  '        continue-on-error: true',
  '        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2',
  '        with:',
  '          name: gpu-vast-runlog-${{ github.run_id }}-${{ github.run_attempt }}',
  '          path: ${{ runner.temp }}/gpu-vast-runlog/runlog.txt',
  '          retention-days: 30',
  '          if-no-files-found: ignore',
  '          overwrite: false',
].join('\n');
