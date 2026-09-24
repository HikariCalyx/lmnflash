use std::time::Duration;
use rusb::UsbContext;

const FB_CLASS: u8 = 0xFF;
const FB_SUBCLASS: u8 = 0x42;
const FB_PROTOCOL: u8 = 0x03;
const FB_VENDORS: &[u16] = &[0x18D1, 0x0451, 0x0502, 0x0FCE, 0x05C6, 0x22B8, 0x0955, 0x413C, 0x2314, 0x0BB4, 0x8087, 0x0489, 0x2E04, 0x0E8D];

/// Return the effective flash size of a file (in bytes).
/// For sparse images this is total_blks * blk_sz; for raw files just the file size.
pub fn flash_size(data: &[u8]) -> usize {
    if data.len() >= 28 && &data[0..4] == b"\x3a\xff\x26\xed" {
        let blk_sz = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let total_blks = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
        total_blks * blk_sz
    } else {
        data.len()
    }
}

/// Return the effective flash size by reading only the file header.
/// Avoids loading the entire file into memory for size calculation.
/// Returns (flash_size, is_sparse).
pub fn flash_size_from_path(path: &std::path::Path) -> std::io::Result<(usize, bool)> {
    let meta = std::fs::metadata(path)?;
    let file_len = meta.len() as usize;
    if file_len < 28 {
        return Ok((file_len, false));
    }
    // Read just the first 28 bytes to check for sparse magic.
    let mut buf = [0u8; 28];
    let mut f = std::fs::File::open(path)?;
    use std::io::Read;
    f.read_exact(&mut buf)?;
    if &buf[0..4] == b"\x3a\xff\x26\xed" {
        let blk_sz = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
        let total_blks = u32::from_le_bytes(buf[16..20].try_into().unwrap()) as usize;
        Ok((total_blks * blk_sz, true))
    } else {
        Ok((file_len, false))
    }
}

pub struct FastbootDevice<'a> {
    handle: rusb::DeviceHandle<rusb::Context>,
    serial: String,
    timeout: Duration,
    max_download: usize,
    /// Sink for every reply the bootloader sends, including the `INFO`/`DATA`
    /// packets the command helpers would otherwise discard (see
    /// [`Self::set_packet_logger`]).
    packet_logger: std::cell::RefCell<Option<Box<dyn Fn(String) + Send + 'a>>>,
}

/// Parses a size reported by `getvar`: `0x…` hex, bare hex (e.g. `1f000000`),
/// or plain decimal.
fn parse_size(value: &str) -> Option<u64> {
    let value = value.trim();

    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok();
    }

    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse().ok();
    }

    u64::from_str_radix(value, 16).ok()
}

/// Renders one reply packet as a readable line, or `None` for the packets
/// that carry no information at all (an empty `OKAY`).
fn format_packet(header: &[u8], payload: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(payload)
        .trim_end_matches('\0')
        .to_string();

    match header {
        b"INFO" | b"FAIL" | b"OKAY" | b"TEXT" => {
            if text.is_empty() {
                None
            } else {
                Some(format!("{} {}", String::from_utf8_lossy(header), text))
            }
        }
        // `DATA<8 hex max-download-size><8 hex size>`: the download the device
        // is willing to accept.
        b"DATA" => Some(format!("DATA {}", describe_data(&text))),
        _ => None,
    }
}

/// Turns the payload of a `DATA` packet into `4096 bytes (max 8192)`.
fn describe_data(text: &str) -> String {
    let text = text.trim();

    if text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let max = u32::from_str_radix(&text[..8], 16).unwrap_or(0);
        let size = u32::from_str_radix(&text[8..], 16).unwrap_or(0);
        return format!("{size} bytes (max {max})");
    }

    text.to_string()
}

fn is_fastboot_iface(d: &rusb::InterfaceDescriptor) -> bool {
    d.class_code() == FB_CLASS && d.sub_class_code() == FB_SUBCLASS && d.protocol_code() == FB_PROTOCOL
}

impl<'a> FastbootDevice<'a> {
    pub fn list_devices() -> Result<Vec<String>, String> {
        let ctx = rusb::Context::new().map_err(|e| format!("USB: {}", e))?;
        let mut serials = Vec::new();
        for device in ctx.devices().map_err(|e| format!("USB: {}", e))?.iter() {
            let dd = match device.device_descriptor() { Ok(d) => d, Err(_) => continue };
            if !FB_VENDORS.contains(&dd.vendor_id()) { continue; }
            let cfg = match device.active_config_descriptor() { Ok(c) => c, Err(_) => continue };
            if !cfg.interfaces().any(|i| i.descriptors().any(|d| is_fastboot_iface(&d))) { continue; }
            let Ok(h) = device.open() else { continue };
            let lang = h.read_languages(Duration::from_secs(1)).ok().and_then(|l| l.into_iter().next());
            let Some(lang) = lang else { continue };
            if let Ok(sn) = h.read_serial_number_string(lang, &dd, Duration::from_secs(1)) {
                serials.push(sn);
            }
        }
        Ok(serials)
    }

    pub fn connect(serial: &str) -> Result<Self, String> {
        let ctx = rusb::Context::new().map_err(|e| format!("USB: {}", e))?;
        for device in ctx.devices().map_err(|e| format!("USB: {}", e))?.iter() {
            let dd = match device.device_descriptor() { Ok(d) => d, Err(_) => continue };
            if !FB_VENDORS.contains(&dd.vendor_id()) { continue; }
            let cfg = match device.active_config_descriptor() { Ok(c) => c, Err(_) => continue };
            for iface in cfg.interfaces() {
                for d in iface.descriptors() {
                    if !is_fastboot_iface(&d) { continue; }
                    let h = device.open().map_err(|e| format!("Open: {}", e))?;
                    let lang = h.read_languages(Duration::from_secs(1)).ok()
                        .and_then(|l| l.into_iter().next()).ok_or("No lang")?;
                    let sn = h.read_serial_number_string(lang, &dd, Duration::from_secs(1))
                        .map_err(|e| format!("Serial: {}", e))?;
                    if sn != serial { continue; }
                    h.claim_interface(d.interface_number()).map_err(|e| format!("Claim: {}", e))?;
                    return Ok(FastbootDevice {
                        handle: h,
                        serial: sn,
                        timeout: Duration::from_secs(100),
                        max_download: 128 * 1024 * 1024,
                        packet_logger: std::cell::RefCell::new(None),
                    });
                }
            }
        }
        Err(format!("Device {} not found", serial))
    }

    pub fn serial(&self) -> &str { &self.serial }

    /// Routes every reply the bootloader sends — `INFO`, `DATA`, `FAIL` and
    /// non-empty `OKAY` packets — to `logger`.
    ///
    /// Most commands only report success or failure, so the text a device
    /// sends along the way (why a command failed, which slot it selected, how
    /// large a download it accepts, …) is dropped. With a logger installed the
    /// packets are handed over as readable lines:
    ///
    /// ```text
    /// INFO (bootloader) max-download-size: 0x1f000000
    /// DATA 4096 bytes (max 8192)
    /// FAIL Command not allowed
    /// ```
    ///
    /// The callback must not call back into the device.
    pub fn set_packet_logger(&mut self, logger: Option<Box<dyn Fn(String) + Send + 'a>>) {
        *self.packet_logger.borrow_mut() = logger;
    }

    pub fn oem_info(&self, command: &str) -> Result<Vec<String>, String> {
        self.write(format!("oem {}", command).as_bytes())?;
        self.read_info()
    }

    /// Like [`Self::oem_info`], but preserves the INFO lines the bootloader
    /// sent *before* a FAILED packet.
    ///
    /// Motorola bootloaders often explain why a command failed via INFO text
    /// (e.g. `Check 'OEM unlocking' in Android Settings > Developer`) that
    /// `oem_info` would otherwise discard. On failure the returned error is
    /// the FAIL payload plus those INFO lines, so callers can tell the
    /// failure reason apart.
    pub fn oem_info_with_fail_details(&self, command: &str) -> Result<Vec<String>, String> {
        self.write(format!("oem {}", command).as_bytes())?;

        let mut lines = Vec::new();
        loop {
            let (h, p) = self.read_packet()?;
            match h.as_slice() {
                b"INFO" => {
                    let text =
                        String::from_utf8_lossy(&p).trim_end_matches('\0').to_string();
                    if !text.is_empty() {
                        lines.push(text);
                    }
                }
                b"OKAY" => {
                    let text =
                        String::from_utf8_lossy(&p).trim_end_matches('\0').to_string();
                    if !text.is_empty() {
                        lines.push(text);
                    }
                    return Ok(lines);
                }
                b"FAIL" => {
                    let fail =
                        String::from_utf8_lossy(&p).trim_end_matches('\0').to_string();
                    if lines.is_empty() {
                        return Err(fail);
                    }
                    return Err(format!("{fail} | {}", lines.join(" | ")));
                }
                _ => {
                    return Err(format!(
                        "Unexpected response: {}",
                        String::from_utf8_lossy(&h)
                    ));
                }
            }
        }
    }

    pub fn oem(&self, command: &str) -> Result<(), String> {
        self.simple_cmd(format!("oem {}", command).as_bytes())?;
        Ok(())
    }

    pub fn getvar(&self, name: &str) -> Result<String, String> {
        let r = self.simple_cmd(format!("getvar:{}", name).as_bytes())?;
        Ok(String::from_utf8_lossy(&r).trim_end_matches('\0').to_string())
    }

    /// Runs `getvar:<name>` and returns the INFO lines the device reported.
    ///
    /// Bootloaders answer a `getvar:` query with an INFO packet carrying the
    /// variable (e.g. `(bootloader) is-userspace: no`) and an empty OKAY, so
    /// [`Self::getvar`] — which reads only the OKAY payload — loses the value.
    pub fn getvar_lines(&self, name: &str) -> Result<Vec<String>, String> {
        self.write(format!("getvar:{}", name).as_bytes())?;
        self.read_info()
    }

    /// Runs `getvar all` and returns every line the device reports
    /// (e.g. `(bootloader) serialno: ZY22ABC`).
    pub fn getvar_all(&self) -> Result<Vec<String>, String> {
        self.write(b"getvar:all")?;
        self.read_info()
    }

    /// Query the device's max-download-size and update the internal limit.
    ///
    /// Bootloaders report the size in an INFO packet (a `getvar:` reply
    /// carries an empty OKAY), so the value has to be read with
    /// [`Self::getvar_lines`]. The result is capped at 256 MiB: the limit is
    /// used to size the re-sparsed buffers in memory, and flashing several
    /// smaller downloads is cheaper than allocating a gigabyte.
    pub fn refresh_max_download(&mut self) {
        const CAP: usize = 256 * 1024 * 1024;

        let Ok(lines) = self.getvar_lines("max-download-size") else {
            return;
        };

        for line in lines {
            // `(bootloader) max-download-size: 0x1f000000`
            let value = match line.rsplit_once(':') {
                Some((_, value)) => value,
                None => line.as_str(),
            };

            if let Some(size) = parse_size(value) {
                if size > 0 {
                    self.max_download = (size as usize).min(CAP);
                    return;
                }
            }
        }
    }

    // Flash data to a partition. Automatically handles sparse images.
    /// `on_progress` is called with (bytes_sent_so_far, total_bytes) after each USB bulk write.
    pub fn flash(&self, partition: &str, data: &[u8], on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        if data.len() >= 4 && &data[0..4] == b"\x3a\xff\x26\xed" {
            return self.flash_sparse_buf(partition, data, on_progress);
        }
        self.download_and_flash(data, on_progress)?;
        // The flash: command may take minutes for large partitions (md4/md5).
        self.simple_cmd_timeout(format!("flash:{}", partition).as_bytes(), Duration::from_secs(600))?;
        Ok(())
    }

    /// Flash a file from disk.  Sparse images are streamed chunk‑by‑chunk
    /// instead of loaded entirely into RAM.
    pub fn flash_file(&self, partition: &str, path: &std::path::Path, on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        let f = std::fs::File::open(path).map_err(|e| format!("Open {}: {}", path.display(), e))?;
        let file_len = f.metadata().map_err(|e| format!("Stat {}: {}", path.display(), e))?.len() as usize;
        self.flash_reader(partition, f, file_len, on_progress)
    }

    /// Flash from any `Read + Seek` source.  Sparse images are streamed;
    /// non‑sparse images are read into memory (they are expected to be small).
    pub fn flash_reader(&self, partition: &str, mut reader: impl std::io::Read + std::io::Seek + Send, file_len: usize, on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        use std::io::SeekFrom;
        // Read first 4 bytes to check magic.
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic).map_err(|e| format!("Read magic: {}", e))?;
        if &magic == b"\x3a\xff\x26\xed" {
            reader.seek(SeekFrom::Start(0)).map_err(|e| format!("Seek: {}", e))?;
            self.flash_sparse_file(partition, reader, file_len, on_progress)
        } else {
            reader.seek(SeekFrom::Start(0)).map_err(|e| format!("Seek: {}", e))?;
            let mut buf = Vec::with_capacity(file_len);
            reader.read_to_end(&mut buf).map_err(|e| format!("Read: {}", e))?;
            self.flash(partition, &buf, on_progress)
        }
    }

    fn flash_sparse_buf(&self, partition: &str, data: &[u8], on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        let mut cursor = std::io::Cursor::new(data);
        self.flash_sparse_file(partition, &mut cursor, data.len(), on_progress)
    }

    fn download_and_flash(&self, data: &[u8], on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        let size_str = format!("{:08x}", data.len());
        self.write(format!("download:{}", size_str).as_bytes())?;

        // Loop until DATA arrives — the device may send INFO/TEXT first.
        let (_h, _) = loop {
            let (h, p) = self.read_packet()?;
            match h.as_slice() {
                b"DATA" => break (h, p),
                b"INFO" | b"TEXT" => { /* keep waiting for DATA */ }
                b"FAIL" => return Err(format!("Download rejected: {}", String::from_utf8_lossy(&p))),
                _ => return Err(format!("Expected DATA, got {}", String::from_utf8_lossy(&h))),
            }
        };

        let chunk = 1024 * 1024;
        let total = data.len();
        for off in (0..total).step_by(chunk) {
            let end = std::cmp::min(off + chunk, total);
            self.handle.write_bulk(self.endpoints()?.0, &data[off..end], self.timeout)
                .map_err(|e| format!("W data: {}", e))?;
            if let Some(ref cb) = on_progress { cb(end, total); }
        }

        // After sending data, the device may send INFO before OKAY.
        loop {
            let (h, p) = self.read_packet()?;
            match h.as_slice() {
                b"OKAY" => break,
                b"INFO" | b"TEXT" => { /* keep waiting */ }
                b"FAIL" => return Err(format!("Flash failed: {}", String::from_utf8_lossy(&p))),
                _ => return Err(format!("Expected OKAY, got {}", String::from_utf8_lossy(&h))),
            }
        }
        Ok(())
    }

    /// Flash an Android sparse image.  Reads chunks from disk on demand
    /// so the whole file is never loaded into RAM.  Re-sparses into splits
    /// that fit the device buffer when necessary.
    /// `on_progress` fires with cumulative output blocks across all splits.
    fn flash_sparse_file(&self, partition: &str, mut reader: impl std::io::Read + std::io::Seek, file_len: usize, on_progress: Option<&dyn Fn(usize, usize)>) -> Result<(), String> {
        use std::io::SeekFrom;
        if file_len < 28 { return Err("Sparse image too short".into()); }

        // Read sparse file header.
        let mut hdr = [0u8; 28];
        reader.read_exact(&mut hdr).map_err(|e| format!("Read header: {}", e))?;
        let file_hdr_sz = u16::from_le_bytes(hdr[8..10].try_into().unwrap()) as usize;
        let chunk_hdr_sz = u16::from_le_bytes(hdr[10..12].try_into().unwrap()) as usize;
        let blk_sz = u32::from_le_bytes(hdr[12..16].try_into().unwrap()) as usize;
        let total_blks = u32::from_le_bytes(hdr[16..20].try_into().unwrap()) as usize;
        let total_chunks = u32::from_le_bytes(hdr[20..24].try_into().unwrap()) as usize;
        let total_output = total_blks * blk_sz;

        let overhead = file_hdr_sz + chunk_hdr_sz;
        let limit = self.max_download.saturating_sub(overhead);

        let mut cumulative: usize = 0;
        let mut chunks_remaining = total_chunks;

        while chunks_remaining > 0 {
            let mut split_chunks: Vec<(u64, usize)> = Vec::new(); // (file_offset, total_sz)
            let mut split_blocks: u32 = 0;
            let mut split_size: usize = 0;
            let start_blk_offset = (cumulative / blk_sz) as u32;

            // Peek at chunk headers; keep those that fit.
            loop {
                if chunks_remaining == 0 { break; }
                let mut ch = [0u8; 12];
                reader.read_exact(&mut ch).map_err(|e| format!("Read chunk hdr: {}", e))?;
                let chunk_blks = u32::from_le_bytes(ch[4..8].try_into().unwrap());
                let total_sz = u32::from_le_bytes(ch[8..12].try_into().unwrap()) as usize;
                let off = reader.stream_position().map_err(|e| format!("Tell: {}", e))? - 12;

                if split_size + total_sz > limit {
                    reader.seek(SeekFrom::Current(-12)).map_err(|e| format!("Back: {}", e))?;
                    break;
                }
                split_chunks.push((off, total_sz));
                split_blocks += chunk_blks;
                split_size += total_sz;
                chunks_remaining -= 1;
                // Skip payload — we re-read it when building.
                let skip = total_sz.saturating_sub(chunk_hdr_sz);
                reader.seek(SeekFrom::Current(skip as i64)).map_err(|e| format!("Skip: {}", e))?;
            }
            if split_chunks.is_empty() {
                // Single chunk exceeds the device buffer — read and split it into
                // limit-sized pieces, each wrapped in its own sparse file download.
                if chunks_remaining == 0 { return Err("Cannot fit any chunk in device buffer".into()); }
                let mut ch = [0u8; 12];
                reader.read_exact(&mut ch).map_err(|e| format!("Read chunk hdr: {}", e))?;
                let chunk_type = u16::from_le_bytes(ch[0..2].try_into().unwrap());
                let _chunk_blks = u32::from_le_bytes(ch[4..8].try_into().unwrap());
                let total_sz = u32::from_le_bytes(ch[8..12].try_into().unwrap()) as usize;
                let data_sz = total_sz.saturating_sub(chunk_hdr_sz);
                chunks_remaining -= 1;

                // Maximum data bytes per piece, aligned to block size.
                let piece_data_max = (limit / blk_sz) * blk_sz;
                let mut data_off = reader.stream_position().map_err(|e| format!("Tell: {}", e))?;
                let mut remaining = data_sz;
                while remaining > 0 {
                    let piece_data = remaining.min(piece_data_max);
                    let piece_blocks = (piece_data / blk_sz) as u32;
                    let piece_total = piece_data + chunk_hdr_sz;
                    let start_blk = (cumulative / blk_sz) as u32;

                    let hdr_blocks = if start_blk > 0 { piece_blocks + start_blk } else { piece_blocks };
                    let n_ch: u32 = if start_blk > 0 { 2 } else { 1 };
                    let mut out = Vec::with_capacity(file_hdr_sz + piece_total + if start_blk > 0 { chunk_hdr_sz } else { 0 });

                    // Sparse file header.
                    out.extend_from_slice(&0xED26FF3Au32.to_le_bytes());
                    out.extend_from_slice(&1u16.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&(file_hdr_sz as u16).to_le_bytes());
                    out.extend_from_slice(&(chunk_hdr_sz as u16).to_le_bytes());
                    out.extend_from_slice(&(blk_sz as u32).to_le_bytes());
                    out.extend_from_slice(&hdr_blocks.to_le_bytes());
                    out.extend_from_slice(&n_ch.to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());

                    if start_blk > 0 {
                        out.extend_from_slice(&0xCAC3u16.to_le_bytes());
                        out.extend_from_slice(&0u16.to_le_bytes());
                        out.extend_from_slice(&start_blk.to_le_bytes());
                        out.extend_from_slice(&(chunk_hdr_sz as u32).to_le_bytes());
                    }

                    // Chunk header + data.
                    out.extend_from_slice(&chunk_type.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&piece_blocks.to_le_bytes());
                    out.extend_from_slice(&(piece_total as u32).to_le_bytes());

                    let mut buf = vec![0u8; piece_data];
                    reader.seek(SeekFrom::Start(data_off)).map_err(|e| format!("Seek piece: {}", e))?;
                    reader.read_exact(&mut buf).map_err(|e| format!("Read piece: {}", e))?;
                    out.extend_from_slice(&buf);

                    self.download_and_flash(&out, None)?;
                    self.simple_cmd_timeout(format!("flash:{}", partition).as_bytes(), Duration::from_secs(600))?;

                    cumulative += piece_blocks as usize * blk_sz;
                    if let Some(ref cb) = on_progress { cb(cumulative, total_output); }

                    remaining -= piece_data;
                    data_off += piece_data as u64;
                }
                // Position past the oversized chunk.
                reader.seek(SeekFrom::Start(data_off)).map_err(|e| format!("Seek past: {}", e))?;
                continue; // next outer iteration
            }

            // Build split file.
            let hdr_blocks = if start_blk_offset > 0 { split_blocks + start_blk_offset } else { split_blocks };
            let n_ch = if start_blk_offset > 0 { split_chunks.len() as u32 + 1 } else { split_chunks.len() as u32 };
            let mut out = Vec::with_capacity(file_hdr_sz + split_size + if start_blk_offset > 0 { chunk_hdr_sz } else { 0 });

            out.extend_from_slice(&0xED26FF3Au32.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&(file_hdr_sz as u16).to_le_bytes());
            out.extend_from_slice(&(chunk_hdr_sz as u16).to_le_bytes());
            out.extend_from_slice(&(blk_sz as u32).to_le_bytes());
            out.extend_from_slice(&hdr_blocks.to_le_bytes());
            out.extend_from_slice(&n_ch.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());

            if start_blk_offset > 0 {
                out.extend_from_slice(&0xCAC3u16.to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&start_blk_offset.to_le_bytes());
                out.extend_from_slice(&(chunk_hdr_sz as u32).to_le_bytes());
            }

            // Read chunks from disk.
            let mut buf = vec![0u8; split_size];
            for (off, sz) in &split_chunks {
                reader.seek(SeekFrom::Start(*off)).map_err(|e| format!("Seek: {}", e))?;
                let b = &mut buf[..*sz];
                reader.read_exact(b).map_err(|e| format!("Read chunk: {}", e))?;
                out.extend_from_slice(b);
            }

            self.download_and_flash(&out, None)?;
            self.simple_cmd_timeout(format!("flash:{}", partition).as_bytes(), Duration::from_secs(600))?;
            drop(out);
            drop(buf);

            cumulative += split_blocks as usize * blk_sz;
            if let Some(ref cb) = on_progress { cb(cumulative, total_output); }

            // Position after last chunk for next iteration.
            if let Some((lo, ls)) = split_chunks.last() {
                reader.seek(SeekFrom::Start(*lo + *ls as u64)).map_err(|e| format!("Seek nxt: {}", e))?;
            }
        }
        Ok(())
    }

    /// Erase a partition.
    pub fn erase(&self, partition: &str) -> Result<(), String> {
        self.simple_cmd_timeout(format!("erase:{}", partition).as_bytes(), Duration::from_secs(600))?;
        Ok(())
    }

    pub fn reboot(&self) -> Result<(), String> { self.simple_cmd(b"reboot")?; Ok(()) }
    pub fn reboot_bootloader(&self) -> Result<(), String> { self.simple_cmd(b"reboot-bootloader")?; Ok(()) }
    pub fn continue_boot(&self) -> Result<(), String> { self.simple_cmd(b"continue")?; Ok(()) }

    fn simple_cmd(&self, cmd: &[u8]) -> Result<Vec<u8>, String> { self.write(cmd)?; self.read_okay() }

    /// Like simple_cmd but with an explicit timeout for the response read.
    /// Loops over INFO messages until OKAY or FAIL arrives.
    fn simple_cmd_timeout(&self, cmd: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
        self.write(cmd)?;
        loop {
            let (h, p) = self.read_packet_with_timeout(timeout)?;
            match h.as_slice() {
                b"OKAY" => return Ok(p),
                b"FAIL" => return Err(String::from_utf8_lossy(&p).to_string()),
                b"INFO" => { /* keep reading for OKAY/FAIL */ }
                _ => return Err(format!("Unexpected response: {}", String::from_utf8_lossy(&h))),
            }
        }
    }

    fn write(&self, data: &[u8]) -> Result<(), String> {
        let (out, _) = self.endpoints()?;
        self.handle.write_bulk(out, data, self.timeout)
            .map_err(|e| format!("W: {}", e))?;
        Ok(())
    }

    fn read_bulk(&self, buf: &mut [u8], timeout: Duration) -> Result<usize, String> {
        let (_, inp) = self.endpoints()?;
        self.handle.read_bulk(inp, buf, timeout).map_err(|e| format!("R: {}", e))
    }

    fn read_packet(&self) -> Result<(Vec<u8>, Vec<u8>), String> {
        self.read_packet_with_timeout(self.timeout)
    }

    /// Reads one reply packet and reports it to the packet logger.
    fn read_packet_with_timeout(&self, timeout: Duration) -> Result<(Vec<u8>, Vec<u8>), String> {
        let mut buf = [0u8; 64];
        let n = self.read_bulk(&mut buf, timeout)?;
        if n < 4 {
            return Err("Short".into());
        }

        let (header, payload) = (buf[..4].to_vec(), buf[4..n].to_vec());
        self.report_packet(&header, &payload);
        Ok((header, payload))
    }

    /// Hands one reply packet to the [packet logger](Self::set_packet_logger),
    /// if one is installed.
    fn report_packet(&self, header: &[u8], payload: &[u8]) {
        let logged = self.packet_logger.borrow();
        if let Some(logger) = logged.as_ref() {
            if let Some(line) = format_packet(header, payload) {
                logger(line);
            }
        }
    }

    fn read_okay(&self) -> Result<Vec<u8>, String> {
        loop {
            let (h, p) = self.read_packet()?;
            match h.as_slice() {
                b"OKAY" => return Ok(p),
                b"FAIL" => return Err(format!("FAIL: {}", String::from_utf8_lossy(&p))),
                b"INFO" => {}
                _ => return Err(format!("?{}", String::from_utf8_lossy(&h))),
            }
        }
    }

    fn read_info(&self) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        loop {
            let (h, p) = self.read_packet()?;
            match h.as_slice() {
                b"INFO" => {
                    let t = String::from_utf8_lossy(&p).trim_end_matches('\0').to_string();
                    if !t.is_empty() { lines.push(t); }
                }
                b"OKAY" => {
                    let t = String::from_utf8_lossy(&p).trim_end_matches('\0').to_string();
                    if !t.is_empty() { lines.push(t); }
                    return Ok(lines);
                }
                b"FAIL" => return Err(format!("FAIL: {}", String::from_utf8_lossy(&p))),
                _ => return Err(format!("?{}", String::from_utf8_lossy(&h))),
            }
        }
    }

    fn endpoints(&self) -> Result<(u8, u8), String> {
        let dev = self.handle.device();
        let cfg = dev.active_config_descriptor().map_err(|e| format!("Cfg: {}", e))?;
        let (mut ep_in, mut ep_out) = (None, None);
        for iface in cfg.interfaces() {
            for d in iface.descriptors() {
                if !is_fastboot_iface(&d) { continue; }
                for ep in d.endpoint_descriptors() {
                    if ep.transfer_type() != rusb::TransferType::Bulk { continue; }
                    match ep.direction() {
                        rusb::Direction::In => ep_in = Some(ep.address()),
                        rusb::Direction::Out => ep_out = Some(ep.address()),
                    }
                }
            }
        }
        match (ep_out, ep_in) { (Some(o), Some(i)) => Ok((o, i)), _ => Err("No endpoints".into()) }
    }
}

impl Drop for FastbootDevice<'_> {
    fn drop(&mut self) { let _ = self.handle.release_interface(0); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_and_fail_keep_their_text() {
        assert_eq!(
            format_packet(b"INFO", b"(bootloader) slot-count: 2\0"),
            Some("INFO (bootloader) slot-count: 2".to_string())
        );
        assert_eq!(
            format_packet(b"FAIL", b"Command not allowed"),
            Some("FAIL Command not allowed".to_string())
        );
    }

    #[test]
    fn silent_packets_are_left_out() {
        // An empty OKAY is what a successful command answers with.
        assert_eq!(format_packet(b"OKAY", b""), None);
        assert_eq!(format_packet(b"OKAY", b"\0\0"), None);
    }

    #[test]
    fn data_packets_are_described() {
        // `DATA<8 hex maximum download size><8 hex requested size>`.
        assert_eq!(
            format_packet(b"DATA", b"0000200000001000"),
            Some("DATA 4096 bytes (max 8192)".to_string())
        );

        // Anything that is not that shape is shown as it came in.
        assert_eq!(
            format_packet(b"DATA", b"whatever"),
            Some("DATA whatever".to_string())
        );
    }
}
