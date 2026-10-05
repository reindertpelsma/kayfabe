/* Compile the real public metadata types; do not restate their definitions. */
#include <stddef.h>
#include <stdio.h>
#include "resource_desc.h"
#include "nvoc/runtime.h"

#define FIELD(type, member) printf("\"" #member "\":%zu,", offsetof(type, member))
#define SIZE(type) printf("\"sizeof\":%zu}", sizeof(type))

int main(void)
{
    /* Some public versions make classId a bit-field: measure initialized bytes. */
    const NVOC_CLASS_INFO class_probe = {.classId = 0x00ffffff};
    const unsigned char *class_bytes = (const unsigned char *)&class_probe;
    printf("{\"RS_RESOURCE_DESC\":{");
    FIELD(RS_RESOURCE_DESC, externalClassId);
    FIELD(RS_RESOURCE_DESC, internalClassId);
    FIELD(RS_RESOURCE_DESC, pClassInfo);
    FIELD(RS_RESOURCE_DESC, allocParamSize);
    FIELD(RS_RESOURCE_DESC, bParamRequired);
    FIELD(RS_RESOURCE_DESC, bMultiInstance);
    FIELD(RS_RESOURCE_DESC, bAnyParent);
    FIELD(RS_RESOURCE_DESC, pParentList);
    FIELD(RS_RESOURCE_DESC, freePriority);
    FIELD(RS_RESOURCE_DESC, flags);
    SIZE(RS_RESOURCE_DESC);
    printf(",\"NVOC_CLASS_INFO\":{");
    FIELD(NVOC_CLASS_INFO, size);
    printf("\"classIdProbe\":[");
    for (size_t i = 0; i < sizeof(class_probe); ++i)
        printf("%s%u", i ? "," : "", (unsigned)class_bytes[i]);
    printf("],");
    SIZE(NVOC_CLASS_INFO);
    printf(",\"NVOC_EXPORTED_METHOD_DEF\":{");
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, pFunc);
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, flags);
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, accessRight);
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, methodId);
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, paramSize);
    FIELD(struct NVOC_EXPORTED_METHOD_DEF, pClassInfo);
    SIZE(struct NVOC_EXPORTED_METHOD_DEF);
    printf("}\n");
    return 0;
}
