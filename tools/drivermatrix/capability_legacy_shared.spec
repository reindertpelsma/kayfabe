# Independent compiler audit of shared control groups omitted by sdk.spec's name filter.
# Run with the existing dm.py against exact NVIDIA tags 535.104.05 and 545.23.06.
# Include all control headers; GCC resolves conditions, aliases and numeric values.
macros legacy_shared_controls sdk nvtypes.h,ctrl/*.h,ctrl/*/*.h (NV00FD|NV9096|NV906F|NV208F|NV90E6|NV_CONF_COMPUTE|NV_SEMAPHORE_SURFACE)_CTRL_[A-Z0-9_]+
