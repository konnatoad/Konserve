#[link(name = "konserve_archiver", kind = "static")]
unsafe extern "C" {
    pub fn konserve_gzip_tar(
        input_tar: *const std::os::raw::c_char,
        output_targz: *const std::os::raw::c_char,
    ) -> i32;
}
