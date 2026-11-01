#include <stdio.h>
#include <stddef.h>
#include "ctrl/ctrl0073/ctrl0073dpu.h"
#include "ctrl/ctrl0073/ctrl0073base.h"
#include "ctrl/ctrl0073/ctrl0073stereo.h"
#include "ctrl/ctrl0073/ctrl0073internal.h"
#include "ctrl/ctrl0073/ctrl0073specific.h"
#include "ctrl/ctrl0073/ctrl0073system.h"
#include "ctrl/ctrl0073/ctrl0073svp.h"
#include "ctrl/ctrl0073/ctrl0073psr.h"
#include "ctrl/ctrl0073/ctrl0073common.h"
#include "ctrl/ctrl0073/ctrl0073event.h"
#include "ctrl/ctrl0073/ctrl0073dp.h"
#include "ctrl/ctrl0073/ctrl0073dfp.h"
#include "ctrl/ctrl2080/ctrl2080internal.h"
int main(void){
printf("SIZE 7537216 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS));
printf("FIELD 7537216 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537216 displayId %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS,displayId),sizeof(((NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS*)0)->displayId));
printf("FIELD 7537216 displayType %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS,displayType),sizeof(((NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS*)0)->displayType));
printf("SIZE 7537221 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS));
printf("FIELD 7537221 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537221 displayId %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS,displayId),sizeof(((NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS*)0)->displayId));
printf("FIELD 7537221 bufferSize %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS,bufferSize),sizeof(((NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS*)0)->bufferSize));
printf("FIELD 7537221 flags %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS,flags),sizeof(((NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS*)0)->flags));
printf("FIELD 7537221 edidBuffer %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS,edidBuffer),sizeof(((NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS*)0)->edidBuffer));
printf("SIZE 7537287 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS));
printf("FIELD 7537287 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537287 headMask %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS,headMask),sizeof(((NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS*)0)->headMask));
printf("SIZE 7537290 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS));
printf("FIELD 7537290 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537290 displayId %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS,displayId),sizeof(((NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS*)0)->displayId));
printf("FIELD 7537290 pclkLimit %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS,pclkLimit),sizeof(((NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS*)0)->pclkLimit));
printf("FIELD 7537290 orPclkLimit %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS,orPclkLimit),sizeof(((NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS*)0)->orPclkLimit));
printf("FIELD 7537290 vbPclkLimit %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS,vbPclkLimit),sizeof(((NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS*)0)->vbPclkLimit));
printf("SIZE 7537291 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS));
printf("FIELD 7537291 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537291 displayId %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,displayId),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->displayId));
printf("FIELD 7537291 index %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,index),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->index));
printf("FIELD 7537291 type %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,type),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->type));
printf("FIELD 7537291 protocol %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,protocol),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->protocol));
printf("FIELD 7537291 ditherType %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,ditherType),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->ditherType));
printf("FIELD 7537291 ditherAlgo %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,ditherAlgo),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->ditherAlgo));
printf("FIELD 7537291 location %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,location),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->location));
printf("FIELD 7537291 rootPortId %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,rootPortId),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->rootPortId));
printf("FIELD 7537291 dcbIndex %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,dcbIndex),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->dcbIndex));
printf("FIELD 7537291 vbiosAddress %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,vbiosAddress),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->vbiosAddress));
printf("FIELD 7537291 bIsLitByVbios %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,bIsLitByVbios),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->bIsLitByVbios));
printf("FIELD 7537291 bIsDispDynamic %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS,bIsDispDynamic),sizeof(((NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS*)0)->bIsDispDynamic));
printf("SIZE 7537316 %zu\n",sizeof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS));
printf("FIELD 7537316 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537316 newDevices %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS,newDevices),sizeof(((NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS*)0)->newDevices));
printf("FIELD 7537316 properties %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS,properties),sizeof(((NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS*)0)->properties));
printf("FIELD 7537316 enable %zu %zu\n",offsetof(NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS,enable),sizeof(((NV0073_CTRL_SPECIFIC_DISPLAY_CHANGE_PARAMS*)0)->enable));
printf("SIZE 7536897 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS));
printf("FIELD 7536897 capsTbl %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS,capsTbl),sizeof(((NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS*)0)->capsTbl));
printf("SIZE 7536898 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS));
printf("FIELD 7536898 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536898 flags %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS,flags),sizeof(((NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS*)0)->flags));
printf("FIELD 7536898 numHeads %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS,numHeads),sizeof(((NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS*)0)->numHeads));
printf("SIZE 7536903 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS));
printf("FIELD 7536903 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536903 displayMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS,displayMask),sizeof(((NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS*)0)->displayMask));
printf("FIELD 7536903 displayMaskDDC %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS,displayMaskDDC),sizeof(((NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS*)0)->displayMaskDDC));
printf("SIZE 7536904 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS));
printf("FIELD 7536904 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536904 flags %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS,flags),sizeof(((NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS*)0)->flags));
printf("FIELD 7536904 displayMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS,displayMask),sizeof(((NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS*)0)->displayMask));
printf("FIELD 7536904 retryTimeMs %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS,retryTimeMs),sizeof(((NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS*)0)->retryTimeMs));
printf("SIZE 7536906 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS));
printf("FIELD 7536906 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536906 flags %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS,flags),sizeof(((NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS*)0)->flags));
printf("FIELD 7536906 hotplugAfterEdidMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS,hotplugAfterEdidMask),sizeof(((NV0073_CTRL_SYSTEM_GET_HOTPLUG_STATE_PARAMS*)0)->hotplugAfterEdidMask));
printf("SIZE 7536907 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS));
printf("FIELD 7536907 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536907 displayMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS,displayMask),sizeof(((NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS*)0)->displayMask));
printf("FIELD 7536907 oldDisplayMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS,oldDisplayMask),sizeof(((NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS*)0)->oldDisplayMask));
printf("FIELD 7536907 oldHeadRoutingMap %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS,oldHeadRoutingMap),sizeof(((NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS*)0)->oldHeadRoutingMap));
printf("FIELD 7536907 headRoutingMap %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS,headRoutingMap),sizeof(((NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS*)0)->headRoutingMap));
printf("SIZE 7536908 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS));
printf("FIELD 7536908 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536908 head %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS,head),sizeof(((NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS*)0)->head));
printf("FIELD 7536908 flags %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS,flags),sizeof(((NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS*)0)->flags));
printf("FIELD 7536908 displayId %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS,displayId),sizeof(((NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS*)0)->displayId));
printf("SIZE 7536918 %zu\n",sizeof(NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS));
printf("FIELD 7536918 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536918 internalDisplaysMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS,internalDisplaysMask),sizeof(((NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS*)0)->internalDisplaysMask));
printf("FIELD 7536918 availableInternalDisplaysMask %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS,availableInternalDisplaysMask),sizeof(((NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS*)0)->availableInternalDisplaysMask));
printf("SIZE 7536940 %zu\n",sizeof(NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS));
printf("FIELD 7536940 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536940 displayId %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS,displayId),sizeof(((NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS*)0)->displayId));
printf("FIELD 7536940 bAddition %zu %zu\n",offsetof(NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS,bAddition),sizeof(((NV0073_CTRL_SYSTEM_VRR_DISPLAY_INFO_PARAMS*)0)->bAddition));
printf("SIZE 7536957 %zu\n",sizeof(NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX_PARAMS));
printf("FIELD 7536957 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536957 muxDisplayMask %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX_PARAMS,muxDisplayMask),sizeof(((NV0073_CTRL_CMD_SYSTEM_QUERY_DISPLAY_IDS_WITH_MUX_PARAMS*)0)->muxDisplayMask));
printf("SIZE 7536971 %zu\n",sizeof(NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT_PARAMS));
printf("FIELD 7536971 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536971 bIsSidebandI2cSupported %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT_PARAMS,bIsSidebandI2cSupported),sizeof(((NV0073_CTRL_CMD_SYSTEM_CHECK_SIDEBAND_I2C_SUPPORT_PARAMS*)0)->bIsSidebandI2cSupported));
printf("SIZE 7536977 %zu\n",sizeof(NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA_PARAMS));
printf("FIELD 7536977 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7536977 bMap %zu %zu\n",offsetof(NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA_PARAMS,bMap),sizeof(((NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA_PARAMS*)0)->bMap));
printf("SIZE 7537409 %zu\n",sizeof(NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS));
printf("FIELD 7537409 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7537409 hEvent %zu %zu\n",offsetof(NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS,hEvent),sizeof(((NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS*)0)->hEvent));
printf("FIELD 7537409 event %zu %zu\n",offsetof(NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS,event),sizeof(((NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS*)0)->event));
printf("FIELD 7537409 action %zu %zu\n",offsetof(NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS,action),sizeof(((NV0073_CTRL_EVENT_SET_NOTIFICATION_PARAMS*)0)->action));
printf("SIZE 7541609 %zu\n",sizeof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS));
printf("FIELD 7541609 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7541609 sorIndex %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,sorIndex),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->sorIndex));
printf("FIELD 7541609 maxLinkRate %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,maxLinkRate),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->maxLinkRate));
printf("FIELD 7541609 dpVersionsSupported %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,dpVersionsSupported),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->dpVersionsSupported));
printf("FIELD 7541609 UHBRSupportedByGpu %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,UHBRSupportedByGpu),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->UHBRSupportedByGpu));
printf("FIELD 7541609 bIsMultistreamSupported %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bIsMultistreamSupported),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bIsMultistreamSupported));
printf("FIELD 7541609 bIsSCEnabled %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bIsSCEnabled),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bIsSCEnabled));
printf("FIELD 7541609 bHasIncreasedWatermarkLimits %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bHasIncreasedWatermarkLimits),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bHasIncreasedWatermarkLimits));
printf("FIELD 7541609 bIsPC2Disabled %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bIsPC2Disabled),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bIsPC2Disabled));
printf("FIELD 7541609 isSingleHeadMSTSupported %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,isSingleHeadMSTSupported),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->isSingleHeadMSTSupported));
printf("FIELD 7541609 bFECSupported %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bFECSupported),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bFECSupported));
printf("FIELD 7541609 bIsTrainPhyRepeater %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bIsTrainPhyRepeater),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bIsTrainPhyRepeater));
printf("FIELD 7541609 bOverrideLinkBw %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bOverrideLinkBw),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bOverrideLinkBw));
printf("FIELD 7541609 bUseRgFlushSequence %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bUseRgFlushSequence),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bUseRgFlushSequence));
printf("FIELD 7541609 bSupportDPDownSpread %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bSupportDPDownSpread),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bSupportDPDownSpread));
printf("FIELD 7541609 bAvoidHBR3 %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bAvoidHBR3),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bAvoidHBR3));
printf("FIELD 7541609 bIsDpTunnelingHwBugWarEnabled %zu %zu\n",offsetof(NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS,bIsDpTunnelingHwBugWarEnabled),sizeof(((NV0073_CTRL_CMD_DP_GET_CAPS_PARAMS*)0)->bIsDpTunnelingHwBugWarEnabled));
printf("SIZE 7541056 %zu\n",sizeof(NV0073_CTRL_DFP_GET_INFO_PARAMS));
printf("FIELD 7541056 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_INFO_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_DFP_GET_INFO_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7541056 displayId %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_INFO_PARAMS,displayId),sizeof(((NV0073_CTRL_DFP_GET_INFO_PARAMS*)0)->displayId));
printf("FIELD 7541056 flags %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_INFO_PARAMS,flags),sizeof(((NV0073_CTRL_DFP_GET_INFO_PARAMS*)0)->flags));
printf("FIELD 7541056 UHBRSupportedByDfp %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_INFO_PARAMS,UHBRSupportedByDfp),sizeof(((NV0073_CTRL_DFP_GET_INFO_PARAMS*)0)->UHBRSupportedByDfp));
printf("SIZE 7541058 %zu\n",sizeof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS));
printf("FIELD 7541058 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7541058 displayId %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS,displayId),sizeof(((NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS*)0)->displayId));
printf("FIELD 7541058 flags %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS,flags),sizeof(((NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS*)0)->flags));
printf("FIELD 7541058 maxTmdsClkRateHz %zu %zu\n",offsetof(NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS,maxTmdsClkRateHz),sizeof(((NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS*)0)->maxTmdsClkRateHz));
printf("SIZE 7541074 %zu\n",sizeof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS));
printf("FIELD 7541074 subDeviceInstance %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,subDeviceInstance),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->subDeviceInstance));
printf("FIELD 7541074 displayId %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,displayId),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->displayId));
printf("FIELD 7541074 sorExcludeMask %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,sorExcludeMask),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->sorExcludeMask));
printf("FIELD 7541074 slaveDisplayId %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,slaveDisplayId),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->slaveDisplayId));
printf("FIELD 7541074 bIs2Head1Or %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,bIs2Head1Or),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->bIs2Head1Or));
printf("FIELD 7541074 sorAssignList %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,sorAssignList),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->sorAssignList));
printf("FIELD 7541074 reservedSorMask %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,reservedSorMask),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->reservedSorMask));
printf("FIELD 7541074 flags %zu %zu\n",offsetof(NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS,flags),sizeof(((NV0073_CTRL_DFP_ASSIGN_SOR_PARAMS*)0)->flags));
printf("SIZE 545262168 %zu\n",sizeof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS));
printf("FIELD 545262168 addressSpace %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,addressSpace),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->addressSpace));
printf("FIELD 545262168 physicalAddr %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,physicalAddr),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->physicalAddr));
printf("FIELD 545262168 limit %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,limit),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->limit));
printf("FIELD 545262168 cacheSnoop %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,cacheSnoop),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->cacheSnoop));
printf("FIELD 545262168 hclass %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,hclass),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->hclass));
printf("FIELD 545262168 channelInstance %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,channelInstance),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->channelInstance));
printf("FIELD 545262168 valid %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,valid),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->valid));
printf("FIELD 545262168 pbTargetAperture %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,pbTargetAperture),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->pbTargetAperture));
printf("FIELD 545262168 channelPBSize %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,channelPBSize),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->channelPBSize));
printf("FIELD 545262168 subDeviceId %zu %zu\n",offsetof(NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS,subDeviceId),sizeof(((NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS*)0)->subDeviceId));
return 0;}
