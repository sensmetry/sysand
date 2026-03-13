package com.sensmetry.sysand.model;

public enum CompressionMethod {
    // Store the files as is
    STORED,
    // Compress the files using Deflate
    DEFLATED,
    /// Compress the files using BZIP2. Only available when sysand is compiled with feature kpar-bzip2
    BZIP2,
    /// Compress the files using ZStandard. Only available when sysand is compiled with feature kpar-zstd
    ZSTD,
    /// Compress the files using XZ. Only available when sysand is compiled with feature kpar-xz
    XZ,
    /// Compress the files using PPMd. Only available when sysand is compiled with feature kpar-ppmd
    PPMD,
}
