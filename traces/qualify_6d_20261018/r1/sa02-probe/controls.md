# Separate guest-input helper controls

MEASURED at candidate-product revision 01a171e63 in `encoder-red.txt`, `encoder-green.txt`, `encoder-mutant.txt`, `encoder-restored.txt` and `software-controls.json`: the candidate source and QEMU binary are unchanged. These tests validate the separate helper's request encoder; they do not establish the chosen-client hardware arm.

| Encoder property | Initial zeroed encoder | Filled encoder | Field writes removed | Restored |
| --- | --- | --- | --- | --- |
| Chosen client carried in root parameters | FAIL | PASS | FAIL — killed | PASS |
| Userspace PID carried independently | FAIL | PASS | FAIL — killed | PASS |
| Full layout keeps OS pointer clear | PASS | PASS | PASS | PASS |
| Total | 1 pass / 2 fail | 3 pass / 0 fail | 1 pass / 2 fail | 3 pass / 0 fail |

MEASURED at candidate-product revision 01a171e63 in `software-controls.json`: the final helper build succeeds; a missing explicit guest guard exits before opening the GPU. The first build attempts failed on separate-helper API usage and missing public generated/PTX assets; the final build succeeds. No product compilation failure is inferred from those helper attempts.

MEASURED at candidate-product revision 01a171e63 in `../sa02-broker6.json`: USER and ADMIN auto-client controls both have kernel=false; the chosen-client requests return different root handles and remain NOT_EXERCISED. Encoding the request correctly does not imply the stock guest kernel accepts the requested handle.
