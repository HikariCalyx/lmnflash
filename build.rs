fn main() {
    #[cfg(windows)]
    {
        // Generate version info from Cargo package version
        let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_default();
        let minor = std::env::var("CARGO_PKG_VERSION_MINOR").unwrap_or_default();
        let patch = std::env::var("CARGO_PKG_VERSION_PATCH").unwrap_or_default();
        let full = format!("{}.{}.{}", major, minor, patch);

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let version_rc = format!(
            r#"#include <winresrc.h>

1 VERSIONINFO
FILEVERSION        {major},{minor},{patch},0
PRODUCTVERSION     {major},{minor},{patch},0
FILEFLAGSMASK      0x3F
FILEFLAGS          0x0
FILEOS             VOS_NT_WINDOWS32
FILETYPE           VFT_APP
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904E4"
        BEGIN
            VALUE "CompanyName",      "Hikari Calyx Tech"
            VALUE "FileDescription",  "LMN Flash - Flashing utility for LMN devices"
            VALUE "FileVersion",      "{full}"
            VALUE "InternalName",     "lmnflash"
            VALUE "LegalCopyright",   "2015-2026 (C) Hikari Calyx Tech. All Rights Reserved."
            VALUE "OriginalFilename", "lmnflash.exe"
            VALUE "ProductName",      "LMN Flash"
            VALUE "ProductVersion",   "{full}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1252
    END
END
"#
        );
        let version_path = std::path::Path::new(&out_dir).join("version.rc");
        std::fs::write(&version_path, version_rc).unwrap();

        // Compile both: icon/manifest from app.rc, version from generated file
        let _ = embed_resource::compile(&version_path, embed_resource::NONE);
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    }
}
