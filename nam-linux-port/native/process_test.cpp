// Linux-only allocation regression test, including Eigen's direct malloc calls.
#include "nam_bridge.h"
#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <new>
#include <string>
#include <unistd.h>

namespace {
bool track = false;
size_t allocations = 0;
}
extern "C" void* __real_malloc(size_t);
extern "C" void* __real_calloc(size_t, size_t);
extern "C" void* __real_realloc(void*, size_t);
extern "C" void* __wrap_malloc(size_t n) {
  if (track) ++allocations;
  return __real_malloc(n);
}
extern "C" void* __wrap_calloc(size_t n, size_t size) {
  if (track) ++allocations;
  return __real_calloc(n, size);
}
extern "C" void* __wrap_realloc(void* p, size_t n) {
  if (track) ++allocations;
  return __real_realloc(p, n);
}
void* operator new(size_t n) {
  if (auto* p = std::malloc(n ? n : 1)) return p;
  throw std::bad_alloc();
}
void* operator new[](size_t n) { return ::operator new(n); }
void operator delete(void* p) noexcept { std::free(p); }
void operator delete[](void* p) noexcept { std::free(p); }
void operator delete(void* p, size_t) noexcept { std::free(p); }
void operator delete[](void* p, size_t) noexcept { std::free(p); }

int main() {
  const auto path = std::filesystem::temp_directory_path()
      / ("nam-allocation-test-" + std::to_string(getpid()) + ".nam");
  char error[2048]{};
  const auto run = [&](const std::string& architecture, const std::string& config,
                       const std::string& weights) {
    {
      std::ofstream file(path);
      file << "{\"version\":\"0.5.4\",\"sample_rate\":48000,\"architecture\":\""
           << architecture << "\",\"config\":" << config << ",\"weights\":[" << weights << "]}";
    }
    NamHandle* handle = nam_create(error, sizeof(error));
    NamInfo info{};
    if (!handle || nam_load(handle, path.c_str(), 48000, 64, &info, error, sizeof(error))) {
      std::fprintf(stderr, "Load failed: %s\n", error);
      nam_free(handle);
      return false;
    }
    std::array<float, 64> input{}, left{}, right{};
    input.fill(0.25f);
    allocations = 0;
    int status = 0;
    track = true;
    for (int block = 0; block < 2000 && status == 0; ++block)
      status = nam_process(handle, input.data(), left.data(), right.data(), 1 + block % 64);
    track = false;
    nam_free(handle);
    if (status || allocations) {
      std::fprintf(stderr, "%s: status %d, allocations %zu\n", architecture.c_str(), status, allocations);
      return false;
    }
    return true;
  };
  bool ok = run("Linear", R"({"receptive_field":1,"bias":false})", "0.5");
  std::string taps = "1";
  for (int i = 1; i < 4096; ++i) taps += ",0";
  ok = run("Linear", R"({"receptive_field":4096,"bias":false})", taps) && ok;
  ok = run("LSTM", R"({"input_size":1,"hidden_size":1,"num_layers":1})",
           "0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0") && ok;
  std::filesystem::remove(path);
  return ok ? 0 : 1;
}
