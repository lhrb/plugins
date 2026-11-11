use std::{
    cell::Cell,
    ffi::{c_char, c_int, c_void, CStr, CString},
    marker::PhantomData,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
};

#[repr(C)]
#[derive(Default)]
struct NamInfo {
    expected_sample_rate: f64,
    input_channels: c_int,
    output_channels: c_int,
    supports_arbitrary_sample_rate: c_int,
}

unsafe extern "C" {
    fn nam_create(error: *mut c_char, error_size: usize) -> *mut c_void;
    fn nam_load(
        handle: *mut c_void,
        path: *const c_char,
        rate: f64,
        max_frames: c_int,
        info: *mut NamInfo,
        error: *mut c_char,
        error_size: usize,
    ) -> c_int;
    fn nam_reset(handle: *mut c_void, error: *mut c_char, error_size: usize) -> c_int;
    fn nam_process(
        handle: *mut c_void,
        input: *const f32,
        left: *mut f32,
        right: *mut f32,
        frames: c_int,
    ) -> c_int;
    fn nam_free(handle: *mut c_void);
}

/// An exclusively owned NAM DSP instance.
///
/// Load, reset and drop on a control/worker thread, never the audio thread.
/// Move to the audio thread only after loading; `process` needs exclusive access.
/// Unknown model sample rates remain unknown (no inferred rate or conversion).
pub struct NamModel {
    handle: NonNull<c_void>,
    info: NamInfo,
    sample_rate: f64,
    max_frames: usize,
    _not_sync: PhantomData<Cell<()>>,
}

// The core owns its DSP state and can move between threads, but cannot be shared.
unsafe impl Send for NamModel {}

impl NamModel {
    /// Load and prewarm an official .nam model or mono/stereo .wav impulse response.
    pub fn load(
        path: impl AsRef<Path>,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<Self, String> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err("Sample rate must be finite and positive".into());
        }
        if max_frames == 0 || max_frames > c_int::MAX as usize {
            return Err("Maximum frame count must be positive and fit in a C int".into());
        }
        let path = CString::new(path.as_ref().as_os_str().as_bytes())
            .map_err(|_| "Model path contains a NUL byte".to_string())?;
        let mut error = [0 as c_char; 2048];
        // SAFETY: caller-owned writable error storage lives through this call.
        let handle = NonNull::new(unsafe { nam_create(error.as_mut_ptr(), error.len()) })
            .ok_or_else(|| error_string(&error))?;
        let mut model = Self {
            handle,
            info: NamInfo::default(),
            sample_rate,
            max_frames,
            _not_sync: PhantomData,
        };
        // SAFETY: unique live handle, valid C path and writable info/error buffers.
        let status = unsafe {
            nam_load(
                model.handle.as_ptr(),
                path.as_ptr(),
                sample_rate,
                max_frames as c_int,
                &mut model.info,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(error_string(&error));
        }
        Ok(model)
    }

    /// Reset and prewarm off the audio thread, preserving the host rate and capacity.
    pub fn reset(&mut self) -> Result<(), String> {
        let mut error = [0 as c_char; 2048];
        // SAFETY: exclusive live handle and valid writable error storage.
        let status = unsafe { nam_reset(self.handle.as_ptr(), error.as_mut_ptr(), error.len()) };
        if status == 0 {
            Ok(())
        } else {
            Err(error_string(&error))
        }
    }

    /// Process mono input into stereo output, duplicating mono model output.
    ///
    /// All three lengths must match and fit within the loaded maximum.
    /// Success performs no adapter allocation; errors use static messages.
    pub fn process(
        &mut self,
        input: &[f32],
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<(), &'static str> {
        if input.len() != left.len() || input.len() != right.len() {
            return Err("NAM input and output buffer lengths must match");
        }
        if input.len() > self.max_frames {
            return Err("NAM block exceeds prepared maximum frame count");
        }
        if input.is_empty() {
            return Ok(());
        }
        // SAFETY: exclusive handle and disjoint, valid slices of checked length.
        match unsafe {
            nam_process(
                self.handle.as_ptr(),
                input.as_ptr(),
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                input.len() as c_int,
            )
        } {
            0 => Ok(()),
            1 => Err("NAM model is unavailable, faulted, or the block is invalid"),
            _ => Err(
                "NAM core failed during processing; reload or reset the model off the audio thread",
            ),
        }
    }

    pub fn input_channels(&self) -> usize {
        self.info.input_channels as usize
    }
    pub fn output_channels(&self) -> usize {
        self.info.output_channels as usize
    }
    /// Training/WAV rate; `None` for old models that do not specify one.
    pub fn expected_sample_rate(&self) -> Option<f64> {
        (self.info.expected_sample_rate > 0.0).then_some(self.info.expected_sample_rate)
    }
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    pub fn supports_arbitrary_sample_rate(&self) -> bool {
        self.info.supports_arbitrary_sample_rate != 0
    }
}

impl Drop for NamModel {
    fn drop(&mut self) {
        // SAFETY: exactly one owner frees this handle, after the final exclusive call.
        unsafe { nam_free(self.handle.as_ptr()) }
    }
}

fn error_string(error: &[c_char]) -> String {
    // SAFETY: buffers start zeroed and the bridge writes bounded NUL-terminated messages.
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
