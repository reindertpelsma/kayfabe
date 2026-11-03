#include <stdio.h>
#include <stddef.h>
#include <nvtypes.h>
#include <ctrl/ctrl2080/ctrl2080gpu.h>
#define S(t) printf("sizeof\t%s\t%zu\n", #t, sizeof(t))
#define F(t,f) printf("field\t%s.%s\t%zu\t%zu\n", #t, #f, offsetof(t,f), sizeof(((t*)0)->f))
#define V(v) printf("value\t%s\t%llu\n", #v, (unsigned long long)(v))
int main(void) {
 S(NV2080_CTRL_GPU_EXEC_REG_OPS_PARAMS);S(NV2080_CTRL_GPU_REG_OP);
 F(NV2080_CTRL_GPU_EXEC_REG_OPS_PARAMS,regOpCount);F(NV2080_CTRL_GPU_EXEC_REG_OPS_PARAMS,regOps);
 F(NV2080_CTRL_GPU_REG_OP,regOp);F(NV2080_CTRL_GPU_REG_OP,regType);F(NV2080_CTRL_GPU_REG_OP,regStatus);F(NV2080_CTRL_GPU_REG_OP,regOffset);F(NV2080_CTRL_GPU_REG_OP,regValueLo);
 V(NV2080_CTRL_CMD_GPU_EXEC_REG_OPS);V(NV2080_CTRL_GPU_REG_OP_READ_32);V(NV2080_CTRL_GPU_REG_OP_TYPE_GLOBAL);V(NV2080_CTRL_GPU_REG_OP_STATUS_SUCCESS);V(NV2080_CTRL_GPU_REG_OP_STATUS_NOACCESS);
 return 0;
}
