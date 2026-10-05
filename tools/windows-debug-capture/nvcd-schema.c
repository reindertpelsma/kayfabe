/* Research-only descriptor exporter. Compile against trusted local OGKM sources. */
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include "nvtypes.h"
#include "prbrt.h"
#include "g_nvdebug_pb.h"
#include "nvcd.h"
#include "rmcd.h"

static const PRB_MSG_DESC *seen[2048];
static size_t count;

static void gather(const PRB_MSG_DESC *msg)
{
    size_t i;
    for (i = 0; i < count; ++i)
        if (seen[i] == msg) return;
    if (count == 2048 || msg->num_fields > 4096) exit(2);
    seen[count++] = msg;
    for (i = 0; i < msg->num_fields; ++i)
        if (msg->fields[i].msg_desc) gather(msg->fields[i].msg_desc);
}

static void string(const char *s)
{
    putchar('"');
    for (; *s; ++s) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') putchar('\\');
        if (c < 32) printf("\\u%04x", c);
        else putchar(c);
    }
    putchar('"');
}

#define CONSTANT(x) printf("\"" #x "\":%u,", (unsigned)(x))
#define OFFSET(t, f) printf("\"" #t "." #f "\":%zu,", offsetof(t, f))
int main(void)
{
    const GUID v1 = GUID_NVCD_DUMP_V1;
    size_t i, j, k;
    gather(NVDEBUG_NVDUMP);
    printf("{\"header_size\":%zu,\"record_size\":%zu,\"protobuf_record_size\":%zu,",
           sizeof(NVCD_HEADER), sizeof(NVCD_RECORD), sizeof(RmProtoBuf_RECORD));
    printf("\"v1_guid\":\"%08x-%04x-%04x-", v1.Data1, v1.Data2, v1.Data3);
    for (i = 0; i < 8; ++i) {
        if (i == 2) putchar('-');
        printf("%02x", v1.Data4[i]);
    }
    printf("\",\"constants\":{");
    CONSTANT(NVCD_SIGNATURE); CONSTANT(RmGroup); CONSTANT(NvcdGroup);
    CONSTANT(EndOfData); CONSTANT(RmProtoBuf_V2); CONSTANT(PRB_IS_PACKED);
    CONSTANT(PRB_DOUBLE); CONSTANT(PRB_FLOAT); CONSTANT(PRB_INT32); CONSTANT(PRB_INT64);
    CONSTANT(PRB_UINT32); CONSTANT(PRB_UINT64); CONSTANT(PRB_SINT32); CONSTANT(PRB_SINT64);
    CONSTANT(PRB_FIXED32); CONSTANT(PRB_FIXED64); CONSTANT(PRB_SFIXED32);
    CONSTANT(PRB_SFIXED64); CONSTANT(PRB_BOOL); CONSTANT(PRB_ENUM); CONSTANT(PRB_STRING);
    CONSTANT(PRB_BYTES); CONSTANT(PRB_MESSAGE);
    printf("\"_end\":0},\"offsets\":{");
    OFFSET(NVCD_HEADER, dwSignature); OFFSET(NVCD_HEADER, gVersion);
    OFFSET(NVCD_HEADER, dwSize); OFFSET(NVCD_HEADER, cCheckSum);
    OFFSET(NVCD_RECORD, cRecordGroup); OFFSET(NVCD_RECORD, cRecordType);
    OFFSET(NVCD_RECORD, wRecordSize); OFFSET(RmProtoBuf_RECORD, dwSize);
    printf("\"_end\":0},\"root\":"); string(NVDEBUG_NVDUMP->name);
    printf(",\"messages\":{");
    for (i = 0; i < count; ++i) {
        const PRB_MSG_DESC *msg = seen[i];
        if (i) putchar(',');
        string(msg->name); printf(":{");
        for (j = 0; j < msg->num_fields; ++j) {
            const PRB_FIELD_DESC *field = msg->fields + j;
            if (j) putchar(',');
            printf("\"%u\":{\"name\":", field->number); string(field->name);
            printf(",\"type\":%u,\"label\":%u,\"flags\":%u,\"message\":",
                   field->opts.typ, field->opts.label, field->opts.flags);
            if (field->msg_desc) string(field->msg_desc->name); else printf("null");
            printf(",\"enum\":{");
            if (field->enum_desc) for (k = 0; k < field->enum_desc->count; ++k) {
                const PRB_ENUM_MAPPING *entry = field->enum_desc->mappings + k;
                if (k) putchar(',');
                printf("\"%d\":", entry->value); string(entry->name);
            }
            printf("}}");
        }
        printf("}");
    }
    printf("}}\n");
    return ferror(stdout) ? 1 : 0;
}
