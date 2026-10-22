// Entry point keeps the owner's exact `node --test ci/vast-broker/test` command portable.
import './auth.test.js';
import './broker.test.js';
import './logs.test.js';
import './workflow.test.js';
import './integration.test.js';
import './client.test.js';
import './eligibility.test.js';
import './reporting.test.js';
import './callback-wiring.test.js';
