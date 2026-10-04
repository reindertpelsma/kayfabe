/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Diagnostic only, NOT the GSP recorder. Loads/unloads without creating a
 * device or scanning RAM. Retains the normal GS driver entry runtime.
 */
#include <ntifs.h>
DRIVER_INITIALIZE DriverEntry;
DRIVER_UNLOAD Unload;
void Unload(PDRIVER_OBJECT driver) { UNREFERENCED_PARAMETER(driver); }
NTSTATUS DriverEntry(PDRIVER_OBJECT driver,PUNICODE_STRING registry_path) {
    OBJECT_ATTRIBUTES attrs; HANDLE key;
    UNICODE_STRING name=RTL_CONSTANT_STRING(L"InitDiagnostic");
    ULONG value[2]={5,STATUS_SUCCESS};
    driver->DriverUnload=Unload;
    InitializeObjectAttributes(&attrs,registry_path,OBJ_KERNEL_HANDLE|OBJ_CASE_INSENSITIVE,NULL,NULL);
    if(NT_SUCCESS(ZwOpenKey(&key,KEY_SET_VALUE,&attrs))) {
        (void)ZwSetValueKey(key,&name,0,REG_BINARY,value,sizeof(value));ZwClose(key);
    }
    return STATUS_SUCCESS;
}
