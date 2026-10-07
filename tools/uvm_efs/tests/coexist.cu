// coexist.cu — an ordinary native host CUDA workload, EFS-unaware, run concurrently with the
// EFS fault experiments to show full host CUDA coexists.
//   - a matmul loop (cuBLAS-free, a plain tiled sgemm) that runs for a fixed wall time and reports
//     iterations/second and a checksum, so a stall or a wrong answer during EFS pressure shows up;
//   - a MANAGED-memory workload with real demand paging: cudaMallocManaged, CPU first-touch, GPU
//     kernel, CPU read-back. On a stock host this takes thousands of replayable faults (the exact
//     path EFS must NOT disturb for a non-opted-in VA space). It is verified every round.
// Prints THROUGHPUT lines and one RESULT. It opens no EFS file and passes no EFS flag: its UVM VA
// space is an ordinary one, so EFS must never divert its faults.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cuda_runtime.h>
#include <time.h>

#define CK(x) do { cudaError_t _e = (x); if (_e != cudaSuccess) { \
    printf("CUDA_FAIL %s -> %s (line %d)\n", #x, cudaGetErrorString(_e), __LINE__); exit(3); } } while (0)

static double now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

#define TILE 16
__global__ void sgemm(const float *A, const float *B, float *C, int N)
{
    __shared__ float As[TILE][TILE], Bs[TILE][TILE];
    int row = blockIdx.y * TILE + threadIdx.y;
    int col = blockIdx.x * TILE + threadIdx.x;
    float acc = 0.f;
    for (int t = 0; t < N / TILE; ++t) {
        As[threadIdx.y][threadIdx.x] = A[row * N + t * TILE + threadIdx.x];
        Bs[threadIdx.y][threadIdx.x] = B[(t * TILE + threadIdx.y) * N + col];
        __syncthreads();
        for (int k = 0; k < TILE; ++k)
            acc += As[threadIdx.y][k] * Bs[k][threadIdx.x];
        __syncthreads();
    }
    C[row * N + col] = acc;
}

__global__ void managed_touch(int *p, int n, int add)
{
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n)
        p[i] += add;
}

int main(int argc, char **argv)
{
    double wall = argc > 1 ? atof(argv[1]) : 8.0;
    const int N = 512;
    size_t bytes = (size_t)N * N * sizeof(float);
    float *A, *B, *C;
    CK(cudaMalloc(&A, bytes));
    CK(cudaMalloc(&B, bytes));
    CK(cudaMalloc(&C, bytes));
    float *hA = (float *)malloc(bytes), *hC = (float *)malloc(bytes);
    for (int i = 0; i < N * N; i++)
        hA[i] = (i % 7) * 0.5f;
    CK(cudaMemcpy(A, hA, bytes, cudaMemcpyHostToDevice));
    CK(cudaMemcpy(B, hA, bytes, cudaMemcpyHostToDevice));
    dim3 blk(TILE, TILE), grd(N / TILE, N / TILE);

    // managed workload sizing: 64 MiB, touched on the CPU first, then the GPU
    const int MN = 16 * 1024 * 1024;
    int *M = NULL;
    CK(cudaMallocManaged(&M, (size_t)MN * sizeof(int)));

    double t0 = now();
    long iters = 0, managed_rounds = 0, managed_bad = 0;
    double checksum = 0;
    while (now() - t0 < wall) {
        for (int r = 0; r < 20; r++) {
            sgemm<<<grd, blk>>>(A, B, C, N);
            ++iters;
        }
        CK(cudaDeviceSynchronize());
        CK(cudaMemcpy(hC, C, bytes, cudaMemcpyDeviceToHost));
        checksum = hC[0] + hC[N * N - 1];

        // demand-paging round: CPU writes every page, GPU adds, CPU verifies.
        for (int i = 0; i < MN; i++)
            M[i] = i + (int)managed_rounds;
        managed_touch<<<(MN + 255) / 256, 256>>>(M, MN, 100);
        CK(cudaDeviceSynchronize());
        for (int i = 0; i < MN; i += 4096)
            if (M[i] != i + (int)managed_rounds + 100)
                ++managed_bad;
        ++managed_rounds;
    }
    double dt = now() - t0;
    printf("THROUGHPUT sgemm_iters=%ld iters_per_s=%.1f managed_rounds=%ld managed_bad=%ld checksum=%.3f wall=%.2f\n",
           iters, iters / dt, managed_rounds, managed_bad, checksum, dt);
    printf("RESULT %s\n", (managed_bad == 0 && iters > 0 && managed_rounds > 0) ? "PASS" : "FAIL");
    return (managed_bad == 0 && iters > 0 && managed_rounds > 0) ? 0 : 1;
}
