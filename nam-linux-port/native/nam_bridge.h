#pragma once
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NamHandle NamHandle;
typedef struct NamInfo {
  double expected_sample_rate; /* -1 means unknown */
  int input_channels;
  int output_channels;
  int supports_arbitrary_sample_rate;
} NamInfo;

/* Control-thread only. All errors are copied into the caller-owned buffer. */
NamHandle* nam_create(char* error, size_t error_size);
int nam_load(NamHandle*, const char* path, double sample_rate, int max_frames,
             NamInfo* info, char* error, size_t error_size);
int nam_reset(NamHandle*, char* error, size_t error_size);
void nam_free(NamHandle*);
/* No bridge allocation on success. 0=success, 1=invalid state/size, 2=core exception.
   Caller provides distinct buffers of at least frames samples. */
int nam_process(NamHandle*, const float* input, float* left, float* right, int frames);

#ifdef __cplusplus
}
#endif
