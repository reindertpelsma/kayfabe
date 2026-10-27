// Owner-approved policy lives in one data file, shared with local drivers.
// badHosts seed 224600: provider faults in runs 40425285335 and 40478005010.
import values from '../defaults.json' with { type: 'json' };
export const DEFAULTS = Object.freeze(values);
