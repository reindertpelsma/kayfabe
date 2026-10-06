/* Compile the public optional software-object ABI; no NVIDIA binary is executed. */
#include <stddef.h>
#include <stdio.h>
#include "class/cl5080.h"
int main(void)
{
    printf("{\"class\":%u,\"size\":%zu,\"align\":%zu,\"notifyCompletion\":%zu}\n",
           NV50_DEFERRED_API_CLASS, sizeof(NV5080_ALLOC_PARAMS),
           _Alignof(NV5080_ALLOC_PARAMS), offsetof(NV5080_ALLOC_PARAMS, notifyCompletion));
    return 0;
}
