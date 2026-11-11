#include "nam_bridge.h"
#include "NAM/get_dsp.h"
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <stdexcept>
#include <vector>

struct NamHandle {
  std::unique_ptr<nam::DSP> dsp;
  std::vector<float> input;
  float* inputs[1]{};
  float* outputs[2]{};
  double sample_rate = 0;
  int max_frames = 0;
  bool faulted = false;
};

namespace {
void error_message(char* error, size_t size, const char* message) noexcept {
  if (error && size) std::snprintf(error, size, "%s", message);
}
}

extern "C" NamHandle* nam_create(char* error, size_t size) {
  try { return new NamHandle; }
  catch (const std::exception& e) { error_message(error, size, e.what()); }
  catch (...) { error_message(error, size, "Unknown exception creating NAM handle"); }
  return nullptr;
}

extern "C" int nam_load(NamHandle* handle, const char* path, double rate, int max_frames,
                         NamInfo* info, char* error, size_t size) {
  try {
    if (!handle || !path || !info || !std::isfinite(rate) || rate <= 0 || max_frames <= 0)
      throw std::invalid_argument("NAM requires a path, positive finite sample rate and positive maximum frame count");
    nam::DspLoadOptions options;
    options.prewarm = false;
    auto dsp = nam::get_dsp(std::filesystem::path(path), options);
    if (!dsp) throw std::runtime_error("NAM core returned no model");
    const int inputs = dsp->NumInputChannels(), outputs = dsp->NumOutputChannels();
    if (inputs != 1 || outputs < 1 || outputs > 2)
      throw std::runtime_error("Only one-input, mono/stereo-output NAM models are supported");
    const double expected = dsp->GetExpectedSampleRate();
    const bool arbitrary_rate = dsp->SupportsArbitrarySampleRate();
    if (!arbitrary_rate && expected > 0 && expected != rate) {
      char message[256];
      std::snprintf(message, sizeof(message),
                    "Model requires %.10g Hz but host uses %.10g Hz; this architecture cannot resample", expected, rate);
      throw std::runtime_error(message);
    }
    std::vector<float> input(static_cast<size_t>(max_frames));
    dsp->SetPrewarmOnReset(true);
    dsp->Reset(rate, max_frames);
    handle->dsp = std::move(dsp);
    handle->input = std::move(input);
    handle->inputs[0] = handle->input.data();
    handle->sample_rate = rate;
    handle->max_frames = max_frames;
    handle->faulted = false;
    *info = {expected, inputs, outputs, arbitrary_rate ? 1 : 0};
    return 0;
  } catch (const std::exception& e) { error_message(error, size, e.what()); }
  catch (...) { error_message(error, size, "Unknown exception loading NAM model"); }
  return 1;
}

extern "C" int nam_reset(NamHandle* handle, char* error, size_t size) {
  try {
    if (!handle || !handle->dsp) throw std::runtime_error("No NAM model is loaded");
    handle->faulted = true;
    handle->dsp->Reset(handle->sample_rate, handle->max_frames);
    handle->faulted = false;
    return 0;
  } catch (const std::exception& e) { error_message(error, size, e.what()); }
  catch (...) { error_message(error, size, "Unknown exception resetting NAM model"); }
  return 1;
}

extern "C" int nam_process(NamHandle* handle, const float* input, float* left, float* right, int frames) {
  if (!handle || !handle->dsp || handle->faulted || frames < 0 || frames > handle->max_frames
      || !input || !left || !right) return 1;
  if (frames == 0) return 0;
  try {
    std::copy_n(input, frames, handle->input.data());
    handle->outputs[0] = left;
    handle->outputs[1] = right;
    handle->dsp->process(handle->inputs, handle->outputs, frames);
    if (handle->dsp->NumOutputChannels() == 1) std::copy_n(left, frames, right);
    return 0;
  } catch (...) {
    // Do not format, allocate, or cross the C boundary with an exception on the audio thread.
    handle->faulted = true;
    return 2;
  }
}

extern "C" void nam_free(NamHandle* handle) {
  // Upstream DSP destructors are noexcept. Free only on the control thread.
  delete handle;
}
