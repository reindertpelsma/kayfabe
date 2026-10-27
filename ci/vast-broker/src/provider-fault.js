// Old payloads can label a failed provider provision host_xid when its watcher
// receipt is incomplete. This only changes fault attribution and destroys the
// box: it never admits Xid evidence, continues a stage, or creates a GPU PASS.
export function reportedProviderFault(result, log = '') {
  if (
    result.verdict !== 'FAIL' ||
    !result.suites.length ||
    !result.suites.every(
      (s) => ['host_xid', 'provision'].includes(s.name) && s.pass === 0 && s.skip === 0,
    )
  )
    return null;
  const text = log.replace(/^\[box\] /gm, '');
  const frames = [
    ...text.matchAll(
      /^KF_FINAL_SUMMARY_BEGIN verdict=FAIL stages=4\n[\s\S]*?^KF_FINAL_SUMMARY_END$/gm,
    ),
  ];
  const frame = frames.at(-1)?.[0];
  if (!frame) return null;
  const stages = frame.split('\n').filter((line) => line.startsWith('stage name='));
  const provision = new RegExp(
    '^stage name=provision rc=[1-9][0-9]* duration_ms=[0-9]+' +
      ' result=FAIL reason=(host_xid|provision)$',
  );
  if (
    stages.length !== 4 ||
    !provision.test(stages[0]) ||
    !['cuda_host', 'merge_bar', 'cuda_guest'].every(
      (name, i) => stages[i + 1] === 'stage name=' + name + ' result=NOT_RUN',
    )
  )
    return null;
  const details = frame
    .split('\n')
    .filter((line) =>
      /^(first_fail stage=provision line=|cause stage=provision log_tail=)/.test(line),
    )
    .join('\n');
  const driver = /\bDRIVER_RC=8\b/.test(details);
  const unusable = /GPU unusable|No devices were found/i.test(details);
  if (/no \/dev\/kvm\b/i.test(details)) return 'provider_kvm_missing';
  if (driver && /RmInitAdapter failed|rm_init_adapter failed/.test(details))
    return 'provider_gpu_unavailable';
  if (/\bNOT_READY\b/.test(details) && unusable) return 'provider_gpu_unavailable';
  if (/GPU unusable/i.test(details)) return 'provider_gpu_unavailable';
  return null;
}
