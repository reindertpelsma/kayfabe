// One-load/one-store compute kernel for E6' N4 launch + fault proof.
// M1: 'out' is a MAPPED buffer -> writes 7.
// M2: 'out' is an UNMAPPED VA  -> global store faults (replayable, shader access).
extern "C" __global__ void k(int *out) {
    out[0] = 7;
}
