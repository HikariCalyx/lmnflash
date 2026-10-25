//! Qualcomm's Sahara and Firehose protocols, spoken over the serial port a
//! phone in EDL mode (`05C6:9008`) exposes.
//!
//! "Inject Firehose" is two protocols back to back. **Sahara** uploads the
//! programmer (`prog_firehose_ddr.elf`, `xbl_s_devprg_ns.melf`, …) into the
//! phone's RAM; the phone then speaks **Firehose**, an XML-over-serial command
//! set, which is what reads the GPT of every physical partition (a UFS phone
//! keeps one per LUN) and what resets the phone afterwards.
//!
//! Only what the Partition Manager needs is implemented: the programmer
//! upload, `<configure>`, the GPT reads and `<power value="reset">`. All of it
//! blocks on the serial port, so the UI runs it on a worker thread.
//!
//! The protocol shapes follow Qualcomm's `qdl` (and its Rust port `qdlrs`):
//! Sahara packets are little-endian, a *melf* programmer is a CPIO `newc`
//! archive whose entries are named `<image id>:<name>`, and Firehose answers
//! every XML command with a `<response value="ACK" />` (or `NAK`) that may be
//! preceded by any number of `<log value="…" />` elements.
//!
//! Every session writes a line-by-line account to the log the caller hands in
//! (`Documents/lmnflash/logs/pmgr_<port>_<moment>.log`): the commands, the
//! phone's answers and log lines, the storage attempts, the partition table it
//! read — and, when something fails, the exact error a user can send on.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serial2::SerialPort;

use crate::flash_log::FlashLog;
use crate::StorageType;

/// How long a read or write may wait for the phone. A connection that has not
/// answered anything in this window is treated as frozen.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How long the boot ROM's phase may wait. The programmer is a different
/// program with different timing, and a phone that cannot honour a
/// `<configure>` for the media it does not have simply says nothing — waiting
/// half a minute for that is wasted time, so the Firehose phase gets a
/// shorter window.
const FIREHOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the phone gets between the end of the Sahara upload and the first
/// Firehose command: the programmer has just been started from RAM and is not
/// listening for commands the moment the upload ends.
const PROGRAMMER_SETTLE: Duration = Duration::from_secs(4);

/// How long a boot ROM gets to answer the mode switch that ends a command-mode
/// session. It usually answers at once; one that says nothing wants the
/// greeting instead (see [`sahara_upload`]).
const MODE_SWITCH_WAIT: Duration = Duration::from_secs(3);

/// The file types the programmer picker offers: a plain MBN, a single-image
/// ELF, and the CPIO archive newer platforms use.
pub(crate) const PROGRAMMER_EXTENSIONS: [&str; 3] = ["mbn", "elf", "melf"];

/// The baud rate a USB CDC serial port is opened with. The phone ignores it
/// (the link is USB, not UART); 115200 is what the reference tools use.
const BAUD_RATE: u32 = 115_200;

// --- Sahara ---------------------------------------------------------------

/// The device greets the host; the body carries its protocol version and the
/// mode it is in.
const SAHARA_HELLO: u32 = 0x01;
/// The host answers the greeting and names the mode it wants.
const SAHARA_HELLO_RESP: u32 = 0x02;
/// The device asks for a slice of an image.
const SAHARA_READ_DATA: u32 = 0x03;
/// The 64-bit form of the same request (Sahara protocol 2.8 and newer, which
/// is what recent boot ROMs speak): the same three fields, each eight bytes.
const SAHARA_READ_DATA_64: u32 = 0x12;
/// The device signals the end of an image.
const SAHARA_END_OF_IMAGE: u32 = 0x04;
/// The host acknowledges the end of an image.
const SAHARA_DONE: u32 = 0x05;
/// The device confirms the end of an image, or asks for the next one.
const SAHARA_DONE_RESP: u32 = 0x06;
/// The host asks the device to reset itself.
const SAHARA_RESET: u32 = 0x07;
/// The device says it is resetting.
const SAHARA_RESET_RESP: u32 = 0x08;
/// The host asks the boot ROM to switch to another mode; a boot ROM that is
/// taking commands has no other way back to uploading.
const SAHARA_SWITCH_MODE: u32 = 0x0c;
/// The device says it is ready for the mode the host asked for — what a boot
/// ROM answers when it is told to take commands instead of an image.
const SAHARA_CMD_READY: u32 = 0x0b;
/// The host asks the boot ROM to run one of its own commands.
const SAHARA_EXECUTE: u32 = 0x0d;
/// The boot ROM answers a command, naming how many bytes it has ready.
const SAHARA_EXECUTE_RESP: u32 = 0x0e;
/// The host asks for those bytes.
const SAHARA_EXECUTE_DATA: u32 = 0x0f;
/// "<?xm": the first four bytes of the Firehose XML a phone sends when a
/// programmer is already running (an earlier injection that never reset it).
const SAHARA_XML: u32 = 0x6d78_3f3c;

const SAHARA_VERSION: u32 = 2;
/// The mode a host that is about to send a programmer asks for. Sahara calls
/// it "image transfer pending": the phone waits for the host to serve it.
const SAHARA_MODE_IMAGE_TX_PENDING: u32 = 0;
/// The mode a host asks for when it wants to run the boot ROM's own commands
/// rather than an upload.
const SAHARA_MODE_COMMAND: u32 = 0x03;

/// The boot ROM's "read the hardware ID" command: eight bytes naming the SoC,
/// the OEM and the model the phone was built for.
const SAHARA_READ_HW_ID: u32 = 0x02;
/// The boot ROM's "read the OEM public key hash" command: the SHA-256 of the
/// root certificate the phone's images are signed against.
const SAHARA_READ_PK_HASH: u32 = 0x03;

/// The largest Sahara packet accepted (the same limit the reference tools
/// use; a hello is 48 bytes, a read request 20).
const SAHARA_MAX_PACKET: u32 = 4096;

/// The header of one CPIO `newc` entry: the magic, thirteen 8-digit ASCII hex
/// fields, then the file name.
const CPIO_MAGIC: &[u8; 6] = b"070701";
const CPIO_HEADER_LEN: usize = 110;
const CPIO_TRAILER: &str = "TRAILER!!!";

// --- Firehose -------------------------------------------------------------

/// A response larger than this is not a Firehose answer; the read stops
/// rather than growing without bound.
const MAX_RESPONSE: usize = 1024 * 1024;

/// How many LUNs a UFS phone is probed for. UFS allows up to 32; the ones a
/// phone actually has answer, the rest refuse the read and end the scan.
const MAX_LUNS: u8 = 32;

/// The payload size `<configure>` advertises, and the most a single command
/// may carry in either direction. Reads are chunked to this, writes are sent
/// in packets of this size, exactly like the reference tool.
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// The sector sizes of the storage media (the defaults `qdl` uses).
fn sector_size(storage: StorageType) -> u32 {
    match storage {
        // Only the concrete media reach this function.
        StorageType::Autodetect => 512,
        StorageType::Emmc => 512,
        StorageType::Nvme => 512,
        StorageType::Ufs => 4096,
        StorageType::SpiNor => 4096,
        StorageType::Nand => 4096,
    }
}

/// The name a `<configure>` command gives the storage. `None` for the
/// autodetect pseudo-storage, which is not a media of its own.
fn memory_name(storage: StorageType) -> Option<&'static str> {
    match storage {
        StorageType::Autodetect => None,
        StorageType::Emmc => Some("emmc"),
        StorageType::Ufs => Some("ufs"),
        StorageType::SpiNor => Some("spinor"),
        StorageType::Nand => Some("nand"),
        StorageType::Nvme => Some("nvme"),
    }
}

/// The media a request is tried against, in order. Autodetect checks eMMC
/// first, then UFS, NVMe, SPI NOR and NAND — the order the page documents.
///
/// A programmer whose file name names a media
/// (`prog_ufs_firehose_8998_ddr.elf`) puts that one first: a phone asked for
/// a storage it does not have may not answer at all (a UFS phone given
/// `emmc` left the host waiting out the whole 30 second timeout before it
/// accepted the next command), so starting with the media the loader is
/// built for is the difference between an immediate answer and half a minute
/// of nothing.
fn candidates(storage: StorageType, programmer: &Path) -> Vec<StorageType> {
    match storage {
        StorageType::Autodetect => {
            let mut order = vec![
                StorageType::Emmc,
                StorageType::Ufs,
                StorageType::Nvme,
                StorageType::SpiNor,
                StorageType::Nand,
            ];

            if let Some(named) = named_storage(programmer) {
                order.retain(|media| *media != named);
                order.insert(0, named);
            }

            order
        }
        concrete => vec![concrete],
    }
}

/// The media a programmer's file name names, if it names one.
fn named_storage(programmer: &Path) -> Option<StorageType> {
    let name = programmer
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase();

    [
        ("spinor", StorageType::SpiNor),
        ("emmc", StorageType::Emmc),
        ("nvme", StorageType::Nvme),
        ("nand", StorageType::Nand),
        ("ufs", StorageType::Ufs),
    ]
    .into_iter()
    .find(|(needle, _)| name.contains(needle))
    .map(|(_, media)| media)
}

/// One row of the partition table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub name: String,
    /// The physical partition (LUN) the GPT was read from.
    pub lun: u8,
    pub start_lba: u64,
    pub end_lba: u64,
    /// The sector size of the storage, so byte offsets do not depend on the
    /// table remembering which media it came from.
    pub sector_size: u32,
}

impl Partition {
    /// The byte the partition starts at.
    pub fn begin_offset(&self) -> u64 {
        self.start_lba.saturating_mul(u64::from(self.sector_size))
    }

    /// The byte just past the partition's end.
    pub fn end_offset(&self) -> u64 {
        self.end_lba
            .saturating_add(1)
            .saturating_mul(u64::from(self.sector_size))
    }

    pub fn size_bytes(&self) -> u64 {
        self.end_offset().saturating_sub(self.begin_offset())
    }
}

/// One partition a dump or a write works on, as the page knows it.
///
/// It carries its own sector size so it can be used without the table it came
/// from, and without a second `configure` once the programmer is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub lun: u8,
    pub start_lba: u64,
    pub end_lba: u64,
    pub sector_size: u32,
}

impl Region {
    /// The region of one row of the table.
    pub fn of(partition: &Partition) -> Self {
        Self {
            name: partition.name.clone(),
            lun: partition.lun,
            start_lba: partition.start_lba,
            end_lba: partition.end_lba,
            sector_size: partition.sector_size,
        }
    }

    /// How many sectors the region holds.
    fn sectors(&self) -> u64 {
        self.end_lba
            .saturating_sub(self.start_lba)
            .saturating_add(1)
    }

    /// How many bytes it holds.
    fn bytes(&self) -> u64 {
        self.sectors().saturating_mul(u64::from(self.sector_size))
    }
}

/// What an injection found: the media the phone really has and every GPT
/// entry of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub storage: StorageType,
    pub partitions: Vec<Partition>,
}

/// Who the phone says it is, read from its boot ROM before a programmer is
/// uploaded.
///
/// A programmer is built — and signed — for one SoC and one OEM, and the boot
/// ROM is the only thing that knows which ones the phone in front of the host
/// really has. It answers with a single 64-bit hardware ID, which carries four
/// values: the model in its low sixteen bits, the OEM in the next sixteen, and
/// the SoC in the twenty-four above them.
///
/// The hash of the OEM's public key is a second answer, and the strongest one:
/// it says which root certificate the phone's images have to be signed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The hardware ID as the boot ROM reported it.
    pub hw_id: u64,
    /// The SoC, e.g. `0x28C0E1` for an MSM8998.
    pub msm_id: u32,
    /// The OEM that signed the phone's images.
    pub oem_id: u16,
    /// The model, as the OEM numbered it.
    pub model_id: u16,
    /// SHA-256 of the OEM's root certificate, when the boot ROM gives it out.
    pub pk_hash: Option<Vec<u8>>,
}

impl Identity {
    /// Reads the four values out of the hardware ID.
    ///
    /// `msm_id` is the top 24 bits of the 32-bit half below the hardware
    /// version, so the top byte of the ID — the hardware version, `0x00` on
    /// every phone this was written for — is left out of it.
    fn of(hw_id: u64) -> Self {
        Self {
            hw_id,
            msm_id: ((hw_id >> 32) & 0x00ff_ffff) as u32,
            oem_id: ((hw_id >> 16) & 0xffff) as u16,
            model_id: (hw_id & 0xffff) as u16,
            pk_hash: None,
        }
    }

    /// The hardware ID in the sixteen hex digits the tools print it as.
    pub fn hw_id_text(&self) -> String {
        format!("{:#018X}", self.hw_id)
    }

    /// The SoC's twenty-four bits, the way a programmer's file name spells
    /// them (`prog_firehose_8996_ddr.elf` is MSM8996).
    pub fn msm_id_text(&self) -> String {
        format!("{:#08X}", self.msm_id)
    }

    pub fn oem_id_text(&self) -> String {
        format!("{:#06X}", self.oem_id)
    }

    pub fn model_id_text(&self) -> String {
        format!("{:#06X}", self.model_id)
    }

    /// The OEM public key hash in hex, `None` when the boot ROM kept it to
    /// itself.
    pub fn pk_hash_text(&self) -> Option<String> {
        let hash = self.pk_hash.as_ref()?;

        Some(hash.iter().map(|byte| format!("{byte:02X}")).collect())
    }

    /// The one line the session log records it as.
    fn log_line(&self) -> String {
        let mut line = format!(
            "sahara: identity {} (msm {}, oem {}, model {})",
            self.hw_id_text(),
            self.msm_id_text(),
            self.oem_id_text(),
            self.model_id_text()
        );

        match &self.pk_hash {
            Some(hash) => {
                line.push_str(", pk hash ");
                line.push_str(&self.pk_hash_text().unwrap_or_default());
                line.push_str(&format!(" ({} bytes)", hash.len()));
            }
            None => line.push_str(", pk hash not reported"),
        }

        line
    }
}

/// One physical partition (LUN) of the storage, as the phone reports it.
///
/// A dump of a whole LUN reads it from its first sector, the partition table
/// included: it is the image a phone's storage can be restored from, not one
/// partition of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lun {
    pub number: u8,
    pub sector_size: u32,
    /// How many sectors the whole partition has.
    pub sectors: u64,
}

impl Lun {
    /// The region a dump of the whole partition reads.
    fn region(&self) -> Region {
        Region {
            name: format!("lun{}", self.number),
            lun: self.number,
            start_lba: 0,
            end_lba: self.sectors.saturating_sub(1),
            sector_size: self.sector_size,
        }
    }

    /// How many bytes the whole partition holds.
    pub fn bytes(&self) -> u64 {
        self.sectors.saturating_mul(u64::from(self.sector_size))
    }
}

/// Uploads `programmer` and reads the partition table of the storage the
/// phone reports.
///
/// `storage` is what the user picked: a concrete media makes the phone
/// configure exactly that one, while [`StorageType::Autodetect`] tries the
/// candidates in [`candidates`] order until one answers with a GPT. `log`
/// receives the detailed account of the session.
///
/// Before the programmer goes in, `confirm` is handed the identity the boot
/// ROM reports and has to say whether the upload may go on: this is the only
/// moment the phone names its SoC, its OEM and its model, and a programmer is
/// built for one of each. Nothing is checked here — the answer is the user's —
/// and a `false` resets the phone and ends the session with `Ok(None)`.
pub fn inject(
    port: &str,
    programmer: &Path,
    storage: StorageType,
    log: &mut FlashLog,
    on_reading: &dyn Fn(),
    confirm: &dyn Fn(Preflight) -> InjectAnswer,
) -> Result<Option<Table>, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!("requested storage: {}", storage_name(storage)));

    let result = inject_session(port, programmer, storage, log, on_reading, confirm);

    match &result {
        Ok(Some(table)) => log.line(format!(
            "done: {} partitions on {}",
            table.partitions.len(),
            storage_name(table.storage)
        )),
        Ok(None) => log.line("stopped: the programmer was not uploaded"),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

/// What the user chose at the identity card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectAnswer {
    /// Go on: upload the programmer the card names.
    Upload,
    /// Another file was picked: read it instead, and ask again. Nothing has
    /// been uploaded yet, so the session simply goes on with the new one.
    Reselect(PathBuf),
    /// Stop here: the phone is reset and nothing is uploaded.
    Cancel,
}

/// The programmer an upload serves, and the file it came from.
///
/// It is kept whole rather than as bytes alone so a pick of another file in the
/// middle of the identity card can replace it without touching the phone.
struct Programmer {
    path: PathBuf,
    images: Vec<Option<Vec<u8>>>,
}

impl Programmer {
    fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            path: path.to_path_buf(),
            images: programmer_images(path)?,
        })
    }

    /// The line the session log opens with.
    fn log_line(&self) -> String {
        let count = self.images.iter().filter(|image| image.is_some()).count();
        let bytes: usize = self.images.iter().flatten().map(Vec::len).sum();

        format!(
            "programmer: {} ({}, {count} image(s))",
            self.path.display(),
            crate::firmware::human_size(bytes as u64)
        )
    }
}

fn inject_session(
    port: &str,
    programmer: &Path,
    storage: StorageType,
    log: &mut FlashLog,
    on_reading: &dyn Fn(),
    confirm: &dyn Fn(Preflight) -> InjectAnswer,
) -> Result<Option<Table>, String> {
    let mut programmer = Programmer::open(programmer)?;
    log.line(programmer.log_line());

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };

    if !sahara_upload(&mut session, &mut programmer, confirm)? {
        return Ok(None);
    }
    session.line("sahara: the programmer was uploaded");

    // The programmer needs a moment before it listens for commands, and the
    // page says what the pause is for.
    on_reading();
    session.line(format!(
        "waiting {} seconds for the programmer to start",
        PROGRAMMER_SETTLE.as_secs()
    ));
    std::thread::sleep(PROGRAMMER_SETTLE);
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;
    session.line(format!(
        "firehose: reads now wait {} seconds",
        FIREHOSE_TIMEOUT.as_secs()
    ));

    // The phone greets the host with a burst of `<log>` lines (often without
    // a `<response>` of its own), so the greeting is not read here: the first
    // command's response reader drains it before the answer it waits for.
    let mut last_error = String::from("the phone did not report any storage");
    for candidate in candidates(storage, &programmer.path) {
        let sector_size = sector_size(candidate);
        session.line(format!(
            "storage {}: configuring ({} byte sectors)",
            storage_name(candidate),
            sector_size
        ));

        if let Err(error) = configure(&mut session, candidate) {
            session.line(format!(
                "storage {}: configure failed: {error}",
                storage_name(candidate)
            ));
            last_error = format!("{}: {error}", storage_name(candidate));
            continue;
        }

        match read_storage_partitions(&mut session, candidate, sector_size) {
            Ok(partitions) => {
                session.line(format!(
                    "storage {}: {} partitions",
                    storage_name(candidate),
                    partitions.len()
                ));
                for partition in &partitions {
                    session.line(format!(
                        "  lun {}: {} start_lba {} end_lba {} ({} bytes)",
                        partition.lun,
                        partition.name,
                        partition.start_lba,
                        partition.end_lba,
                        partition.size_bytes()
                    ));
                }

                return Ok(Some(Table {
                    storage: candidate,
                    partitions,
                }));
            }
            Err(error) => {
                session.line(format!(
                    "storage {}: reading the table failed: {error}",
                    storage_name(candidate)
                ));
                last_error = format!("{}: {error}", storage_name(candidate));
            }
        }
    }

    Err(last_error)
}

/// What an injection asks the user to look at before the programmer goes in.
///
/// The two sides of the same question: the boot ROM says which root key the
/// phone is fused for, and the programmer carries the one it is signed
/// against. They have to be the same key — unless the phone is unfused, in
/// which case it checks no signature at all and any programmer will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    /// Who the boot ROM says the phone is.
    pub identity: Identity,
    /// The programmer that is waiting behind the card.
    pub programmer: PathBuf,
    /// The hash of the root certificate the programmer is signed against, or
    /// why it could not be read out of the file.
    pub programmer_hash: Result<Vec<u8>, String>,
}

/// The root keys Qualcomm's own test signing uses. A phone that reports either
/// of them has no fused key of its own: secure boot is off, and the boot ROM
/// accepts a programmer signed with anything, so the two keys cannot disagree.
const TEST_ROOTS: [[u8; 32]; 2] = [
    [
        0xcc, 0x31, 0x53, 0xa8, 0x02, 0x93, 0x93, 0x9b, 0x90, 0xd0, 0x2d, 0x3b, 0xf8, 0xb2, 0x3e,
        0x02, 0x92, 0xe4, 0x52, 0xfe, 0xf6, 0x62, 0xc7, 0x49, 0x98, 0x42, 0x1a, 0xda, 0xd4, 0x2a,
        0x38, 0x0f,
    ],
    [
        0x7b, 0xe4, 0x9b, 0x72, 0xf9, 0xe4, 0x33, 0x72, 0x23, 0xcc, 0xb8, 0x4d, 0x6e, 0xcc, 0xca,
        0x4e, 0x61, 0xce, 0x16, 0xe3, 0x60, 0x2a, 0xc2, 0x00, 0x8c, 0xb1, 0x8b, 0x75, 0xba, 0xbe,
        0x6d, 0x09,
    ],
];

impl Preflight {
    /// Whether the phone is unfused — that is, whether it reported one of
    /// Qualcomm's test root keys instead of a real one.
    ///
    /// An unfused boot ROM verifies no signature, so a programmer that is
    /// signed against a different key still runs; the two hashes are not
    /// supposed to match there.
    pub fn unfused(&self) -> bool {
        let Some(phone) = self.identity.pk_hash.as_deref() else {
            return false;
        };

        TEST_ROOTS
            .iter()
            .any(|root| root.as_slice() == phone)
    }

    /// Whether the programmer is signed against the key the phone is fused for.
    ///
    /// `None` when the two cannot be compared — the programmer's chain could
    /// not be read, or the boot ROM kept its own hash back.
    pub fn roots_match(&self) -> Option<bool> {
        let phone = self.identity.pk_hash.as_ref()?;
        let programmer = self.programmer_hash.as_ref().ok()?;

        Some(phone == programmer)
    }

    /// The programmer's root key hash in hex, `None` when it could not be
    /// read.
    pub fn programmer_hash_text(&self) -> Option<String> {
        let hash = self.programmer_hash.as_ref().ok()?;

        Some(hash.iter().map(|byte| format!("{byte:02X}")).collect())
    }
}

/// What the page's Reboot menu asks the phone to do before it resets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootAction {
    /// Just reset it.
    Normal,
    /// Clear FRP and the config partition, so the OEM-unlocking switch can be
    /// turned on.
    OemUnlock,
    /// Clear FRP/config and have recovery wipe the user data.
    FactoryReset,
    /// Reboot into recovery.
    Recovery,
    /// Reboot into fastbootd.
    Fastbootd,
}

impl RebootAction {
    /// What the log calls the action.
    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal reboot",
            Self::OemUnlock => "enable OEM unlocking",
            Self::FactoryReset => "factory reset",
            Self::Recovery => "recovery",
            Self::Fastbootd => "fastbootd",
        }
    }
}

/// The byte an OEM unlock leaves in the last byte of FRP and `config`: one
/// means "the data those partitions hold is gone".
const OEM_UNLOCK_FLAG: u8 = 0x01;

/// The command the bootloader reads from the start of `misc` (AOSP's
/// `bootloader_message`, whose first field is 32 bytes).
const BCB_COMMAND: &[u8] = b"boot-recovery";

/// The commands recovery runs for a factory reset, and for a boot into
/// fastbootd — the two lines AOSP writes for `--wipe_data` and `--fastboot`.
const WIPE_DATA_COMMANDS: &str = "recovery\n--wipe_data\n";
const FASTBOOT_COMMANDS: &str = "recovery\n--fastboot\n";

/// The first block of `misc`: a 32-byte command, a 32-byte status, then the
/// commands recovery runs.
///
/// The reference images the page's actions were modelled on are exactly the
/// command, a zeroed status and those arguments (84/85 bytes), so the image
/// written here is the same length and everything after it stays untouched.
fn bootloader_message(commands: &str) -> Vec<u8> {
    let mut message = vec![0_u8; 64];
    message[..BCB_COMMAND.len()].copy_from_slice(BCB_COMMAND);
    message.extend_from_slice(commands.as_bytes());
    message
}

/// One thing a reboot does to the storage before it resets the phone.
enum Step {
    /// Clears a whole partition.
    Erase(Region),
    /// Writes an image into a partition.
    Write(Region, Vec<u8>),
    /// Writes zeroes over a whole partition, with `last` in its final byte.
    Blank(Region, u8),
}

impl Step {
    /// How many bytes the step moves, rounded up to whole sectors — the amount
    /// the progress bar counts.
    fn bytes(&self) -> u64 {
        match self {
            Step::Erase(region) | Step::Blank(region, _) => region.bytes(),
            Step::Write(region, data) => padded(data.len() as u64, region.sector_size),
        }
    }
}

/// A length rounded up to whole sectors: what a `<program>` actually carries.
fn padded(length: u64, sector_size: u32) -> u64 {
    length.div_ceil(u64::from(sector_size)) * u64::from(sector_size)
}

/// The steps an action runs, in order.
///
/// `frp` and `config` hold the OEM-unlocking lock state and the carrier
/// configuration; `misc` carries the command the bootloader hands to recovery.
fn reboot_steps(action: RebootAction, partitions: &[Partition]) -> Result<Vec<Step>, String> {
    let find = |name: &str| {
        partitions
            .iter()
            .find(|partition| partition.name.eq_ignore_ascii_case(name))
            .map(Region::of)
    };
    let misc = || {
        find("misc").ok_or_else(|| "this phone has no misc partition".to_owned())
    };

    match action {
        RebootAction::Normal => Ok(Vec::new()),
        RebootAction::OemUnlock => {
            let mut steps = Vec::new();
            for name in ["frp", "config"] {
                if let Some(region) = find(name) {
                    steps.push(Step::Blank(region, OEM_UNLOCK_FLAG));
                }
            }
            if steps.is_empty() {
                return Err("this phone has neither an frp nor a config partition".to_owned());
            }
            Ok(steps)
        }
        RebootAction::FactoryReset => {
            let mut steps = Vec::new();
            for name in ["frp", "config"] {
                if let Some(region) = find(name) {
                    steps.push(Step::Erase(region));
                }
            }
            steps.push(Step::Write(
                misc()?,
                bootloader_message(WIPE_DATA_COMMANDS),
            ));
            Ok(steps)
        }
        RebootAction::Recovery => Ok(vec![Step::Write(misc()?, bootloader_message(""))]),
        RebootAction::Fastbootd => Ok(vec![Step::Write(
            misc()?,
            bootloader_message(FASTBOOT_COMMANDS),
        )]),
    }
}

/// Runs a reboot mode on the phone that is in Firehose mode on `port`.
///
/// Nothing is uploaded or configured again: the programmer is still in RAM, so
/// the session reopens the port, writes what the mode needs into the storage
/// and asks the phone to reset.
pub fn reboot(
    port: &str,
    partitions: &[Partition],
    action: RebootAction,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!("action: {}", action.label()));

    let result = reboot_session(port, partitions, action, log, on_progress);

    match &result {
        Ok(()) => log.line("done: the phone is resetting"),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn reboot_session(
    port: &str,
    partitions: &[Partition],
    action: RebootAction,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let steps = reboot_steps(action, partitions)?;

    // A plain reboot has nothing to write.
    if steps.is_empty() {
        return reset_phone(port, log);
    }

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let overall_total = steps.iter().map(Step::bytes).sum::<u64>();
    let mut before = 0_u64;
    for step in &steps {
        match step {
            Step::Erase(region) => {
                session.line(format!(
                    "erasing {} (lun {}, sectors {}..{})",
                    region.name, region.lun, region.start_lba, region.end_lba
                ));
                let response = request(&mut session, &erase_xml(region))?;
                if !response.is_ack() {
                    return Err(format!(
                        "{}: {}",
                        region.name,
                        refusal(&response, "erase the partition")
                    ));
                }

                before = before.saturating_add(step.bytes());
                on_progress(Progress {
                    name: region.name.clone(),
                    done: region.bytes(),
                    total: region.bytes(),
                    overall_done: before,
                    overall_total,
                });
            }
            Step::Write(region, data) => {
                session.line(format!(
                    "writing {} bytes to {} (lun {}, sectors {}..{})",
                    data.len(),
                    region.name,
                    region.lun,
                    region.start_lba,
                    region.end_lba
                ));
                program_image(
                    &mut session,
                    region,
                    &mut Image::Bytes(data),
                    before,
                    overall_total,
                    on_progress,
                )?;
                before = before.saturating_add(step.bytes());
            }
            Step::Blank(region, last) => {
                session.line(format!(
                    "blanking {} ({} bytes, last byte 0x{last:02X})",
                    region.name,
                    region.bytes()
                ));
                program_image(
                    &mut session,
                    region,
                    &mut Image::Blank { last: *last },
                    before,
                    overall_total,
                    on_progress,
                )?;
                before = before.saturating_add(step.bytes());
            }
        }
    }

    // The phone leaves Firehose mode with the reset, which is the point of
    // every one of the modes.
    request_reset(&mut session)
}

/// Resets the phone that is in Firehose mode on `port`.
///
/// Nothing needs to be uploaded again: the programmer is still in RAM, so the
/// page only has to reopen the port and send `<power value="reset">`.
fn reset_phone(port: &str, log: &mut FlashLog) -> Result<(), String> {
    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };

    request_reset(&mut session)
}

/// Asks the phone behind `session` to reset, and waits for its answer.
fn request_reset(session: &mut Session<'_>) -> Result<(), String> {
    send_xml(
        session,
        "<data><power value=\"reset\" DelayInSeconds=\"0\" /></data>",
    )?;

    match read_response(session) {
        Ok(response) if response.is_ack() => Ok(()),
        Ok(response) => Err(refusal(&response, "reset the phone")),
        Err(error) => {
            // A phone that reboots before it answers is still rebooting; only
            // the write itself has to succeed. The failed read is still worth
            // its own line in the log.
            session.line(format!("note: no answer after the reset command: {error}"));
            Ok(())
        }
    }
}

/// The name of a media, for the error a failed detection reports.
fn storage_name(storage: StorageType) -> &'static str {
    memory_name(storage).unwrap_or("autodetect")
}

/// Where a dump writes its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpTarget {
    /// A folder: one `<name>.img` per partition, numbered when that name is
    /// already taken.
    Folder(PathBuf),
    /// One named file, which is what a save dialog gives back when the table
    /// has a single selected row.
    File(PathBuf),
}

impl DumpTarget {
    /// The folder or file the dump writes to, for the log and the status line.
    pub fn path(&self) -> &Path {
        match self {
            DumpTarget::Folder(path) | DumpTarget::File(path) => path,
        }
    }
}

/// How far a job is, for the page's progress bar.
///
/// `done` and `total` are the partition being worked on; `overall_done` and
/// `overall_total` count every selected partition, which is what the bar under
/// the table shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    /// The partition the report is about.
    pub name: String,
    /// Bytes of that partition done, and how many it has.
    pub done: u64,
    pub total: u64,
    /// Bytes of every selected partition done, and how many they have.
    pub overall_done: u64,
    pub overall_total: u64,
}

impl Progress {
    /// How far the whole job is, 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.overall_total == 0 {
            // Nothing measurable has happened yet: a bar that stands still
            // reads better than one that jumps to full.
            0.0
        } else {
            (self.overall_done as f32 / self.overall_total as f32).clamp(0.0, 1.0)
        }
    }
}

/// Reads partitions of a phone that is still in Firehose mode into `target`.
///
/// Nothing is uploaded or configured again: the programmer keeps running after
/// the injection's port was closed, so this only reopens the port and issues
/// `<read>` commands. `on_progress` is told the partition, its bytes so far
/// and its total size after every chunk.
pub fn dump(
    port: &str,
    regions: &[Region],
    target: &DumpTarget,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<Vec<PathBuf>, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!(
        "action: dump {} partition(s) to {}",
        regions.len(),
        target.path().display()
    ));

    let result = dump_session(port, regions, target, log, on_progress);

    match &result {
        Ok(paths) => log.line(format!("done: {} file(s) written", paths.len())),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

/// The file each partition goes to, in the order of `regions`.
fn dump_paths(regions: &[Region], target: &DumpTarget) -> Result<Vec<PathBuf>, String> {
    match target {
        DumpTarget::Folder(directory) => Ok(regions
            .iter()
            .map(|region| unique_path(directory, &region.name))
            .collect()),
        DumpTarget::File(path) if regions.len() == 1 => Ok(vec![path.clone()]),
        DumpTarget::File(_) => Err(format!(
            "one file holds one partition, but {} are selected",
            regions.len()
        )),
    }
}

fn dump_session(
    port: &str,
    regions: &[Region],
    target: &DumpTarget,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<Vec<PathBuf>, String> {
    let paths = dump_paths(regions, target)?;

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let overall_total = total_bytes(regions);
    let mut before = 0_u64;
    let mut written = Vec::new();
    for (region, path) in regions.iter().zip(&paths) {
        session.line(format!(
            "dumping {} (lun {}, sectors {}..{}) to {}",
            region.name,
            region.lun,
            region.start_lba,
            region.end_lba,
            path.display()
        ));

        if let Err(error) = write_region(&mut session, region, path, before, overall_total, on_progress)
        {
            // A half-written file is not a dump the user can use; leave the
            // folder with only the partitions that went through.
            let _ = std::fs::remove_file(path);
            return Err(format!("{}: {error}", region.name));
        }

        before = before.saturating_add(region.bytes());
        written.push(path.clone());
    }

    Ok(written)
}

/// The bytes every region holds together.
fn total_bytes(regions: &[Region]) -> u64 {
    regions.iter().map(Region::bytes).fold(0_u64, u64::saturating_add)
}

/// What the phone says about its storage, read out of its answer.
///
/// `<getstorageinfo>` is the command a “Flash Info” panel is made of, and every
/// programmer shapes its answer differently: some send one `<response>` full of
/// attributes, others a burst of `<log>` lines — an eMMC phone names its blocks,
/// a UFS phone adds a line per LUN property. Both are read into the same list
/// of facts, and the answer itself is kept beside them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StorageInfo {
    pub luns: Vec<LunInfo>,
}

impl StorageInfo {
    /// The whole answer, as the phone sent it.
    pub fn raw(&self) -> String {
        self.luns
            .iter()
            .map(|lun| format!("LUN {}\n{}", lun.number, lun.raw))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The facts, one per line, the way the card lists them.
    pub fn text(&self) -> String {
        self.luns
            .iter()
            .map(|lun| {
                let mut text = format!("LUN {}", lun.number);

                for (label, value) in &lun.facts {
                    text.push('\n');
                    text.push_str(label);
                    text.push_str(": ");
                    text.push_str(value);
                }

                text
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// One physical partition, as the phone described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunInfo {
    pub number: u8,
    /// What the phone named, in the order it named it: a label and its value.
    pub facts: Vec<(String, String)>,
    /// The XML it answered this partition with.
    pub raw: String,
}

/// What the phone says about its storage, read out of the XML it answers with.
pub fn flash_info(port: &str, log: &mut FlashLog) -> Result<StorageInfo, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line("action: read the storage information");

    let result = flash_info_session(port, log);

    match &result {
        Ok(info) => log.line(format!(
            "done: {} physical partition(s), {} fact(s)",
            info.luns.len(),
            info.luns.iter().map(|lun| lun.facts.len()).sum::<usize>()
        )),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn flash_info_session(port: &str, log: &mut FlashLog) -> Result<StorageInfo, String> {
    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let mut info = StorageInfo::default();
    for lun in 0..MAX_LUNS {
        let xml = format!("<data><getstorageinfo physical_partition_number=\"{lun}\" /></data>");
        send_xml(&mut session, &xml)?;

        // The answer is read whole, not just the `<response>` in it: a
        // programmer may report its storage as a burst of `<log>` lines, which
        // is exactly what a "Flash Info" panel shows.
        let (conversation, response) = read_conversation(&mut session)?;

        if !response.is_ack() {
            // The first partition the phone does not have ends the scan; the
            // first one has to answer.
            if lun == 0 {
                return Err(refusal(&response, "read the storage information"));
            }

            session.line(format!("lun {lun}: not available; the scan ends here"));
            break;
        }

        let facts = storage_facts(&conversation, &response);
        for (label, value) in &facts {
            session.line(format!("  {label}: {value}"));
        }

        info.luns.push(LunInfo {
            number: lun,
            facts,
            raw: split_batches(&conversation),
        });
    }

    if info.luns.is_empty() {
        return Err("the phone reported no storage".to_owned());
    }

    Ok(info)
}

/// One `<data>` element per line.
///
/// The phone sends its answer as a stream, so one element follows the next
/// without even a newline between them; breaking it there keeps every line of
/// the XML readable without changing any of it.
fn split_batches(text: &str) -> String {
    text.replace("</data><?xml", "</data>\n<?xml")
}

/// The facts one answer carries, in the order the phone named them.
///
/// A `<log>` line of the form `label: value` is one, and so is every attribute
/// of the `<response>` element — the two ways programmers report. A line that
/// is a JSON object (the `storage_info` blob a UFS phone sends) is expanded into
/// its keys rather than shown as one long value.
fn storage_facts(conversation: &str, response: &Response) -> Vec<(String, String)> {
    let mut facts = Vec::new();

    for line in attributes(conversation, "log", "value") {
        let line = line.trim();
        let line = line.strip_prefix("INFO: ").unwrap_or(line);

        if let Some(expanded) = expand_json(line) {
            facts.extend(expanded);
            continue;
        }

        // Prose without a `label: value` shape (the ZLP notice, say) carries no
        // fact, and would only get in the way of the ones that do.
        if let Some((label, value)) = line.split_once(':') {
            let label = label.trim();

            if !label.is_empty() {
                facts.push((label.to_owned(), value.trim().to_owned()));
            }
        }
    }

    for (name, value) in &response.attributes {
        if !name.eq_ignore_ascii_case("value") {
            facts.push((name.clone(), value.clone()));
        }
    }

    facts
}

/// The keys of a JSON object, one fact each, when `text` is one.
fn expand_json(text: &str) -> Option<Vec<(String, String)>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;

    let mut facts = Vec::new();
    for (key, value) in object {
        match value {
            // `{"storage_info": {...}}` is one level of wrapping around the
            // report itself, so its keys are what the phone is naming.
            serde_json::Value::Object(inner) => {
                for (name, value) in inner {
                    facts.push((name.clone(), json_text(value)));
                }
            }
            other => facts.push((key.clone(), json_text(other))),
        }
    }

    (!facts.is_empty()).then_some(facts)
}

/// One JSON value as text, without the quotes around a string.
fn json_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The physical partitions (LUNs) the storage has, as the phone reports them.
///
/// The page asks for this before it offers the LUN picker, so every partition
/// is named with the size it really has. `partitions` is the table the
/// injection read: a programmer that does not answer `<getstorageinfo>` for a
/// LUN still has the extent of the partitions read from it.
pub fn storage_luns(
    port: &str,
    partitions: &[Partition],
    log: &mut FlashLog,
) -> Result<Vec<Lun>, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line("action: read the storage layout");

    let result = storage_luns_session(port, partitions, log);

    match &result {
        Ok(luns) => log.line(format!("done: {} physical partition(s)", luns.len())),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn storage_luns_session(
    port: &str,
    partitions: &[Partition],
    log: &mut FlashLog,
) -> Result<Vec<Lun>, String> {
    // Every partition carries the sector size of the storage it was read from,
    // and one storage has one sector size.
    let sector_size = partitions
        .first()
        .map_or(4096, |partition| partition.sector_size);

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let mut luns = Vec::new();
    for number in 0..MAX_LUNS {
        let reported = storage_info(&mut session, number, sector_size);

        let sectors = match (&reported, table_extent(partitions, number)) {
            (Ok(sectors), _) if *sectors > 0 => *sectors,
            (_, Some(sectors)) => {
                if let Err(error) = &reported {
                    session.line(format!(
                        "lun {number}: the phone did not report its size ({error}); \
taking the end of its last partition"
                    ));
                }
                sectors
            }
            (Err(error), None) if number == 0 => {
                return Err(format!("could not read the size of the storage: {error}"));
            }
            (Err(error), None) => {
                // The LUNs are numbered from zero: the first one that does not
                // answer is the first one the phone does not have.
                session.line(format!(
                    "lun {number}: not available ({error}); the scan ends here"
                ));
                break;
            }
            (Ok(_), None) => break,
        };

        session.line(format!(
            "lun {number}: {sectors} sectors ({})",
            crate::firmware::human_size(sectors.saturating_mul(u64::from(sector_size)))
        ));
        luns.push(Lun {
            number,
            sector_size,
            sectors,
        });
    }

    Ok(luns)
}

/// Dumps whole physical partitions into `target`.
///
/// `luns` is what the user picked in the page's LUN picker; `None` dumps every
/// partition the storage has, which is what a media with a single one (eMMC,
/// NVMe, SPI NOR, NAND) offers and what "Dump Entire Storage" means there.
///
/// A dump of a LUN starts at its first sector, so the image holds the
/// partition table as well as the partitions.
pub fn dump_storage(
    port: &str,
    partitions: &[Partition],
    luns: Option<&[Lun]>,
    target: &DumpTarget,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<Vec<PathBuf>, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!(
        "action: dump {} to {}",
        match luns {
            Some(luns) => format!("{} physical partition(s)", luns.len()),
            None => "the whole storage".to_owned(),
        },
        target.path().display()
    ));

    let result = dump_storage_session(port, partitions, luns, target, log, on_progress);

    match &result {
        Ok(paths) => log.line(format!("done: {} file(s) written", paths.len())),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn dump_storage_session(
    port: &str,
    partitions: &[Partition],
    luns: Option<&[Lun]>,
    target: &DumpTarget,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<Vec<PathBuf>, String> {
    // The list the picker showed was read from this phone; a media with one
    // LUN is asked about only now, when the dump starts.
    let luns = match luns {
        Some(luns) => luns.to_vec(),
        None => storage_luns_session(port, partitions, log)?,
    };
    if luns.is_empty() {
        return Err("there is no physical partition to dump".to_owned());
    }

    let regions: Vec<Region> = luns.iter().map(Lun::region).collect();

    dump_session(port, &regions, target, log, on_progress)
}

/// One whole-storage image and the physical partition it is written back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunImage {
    pub path: PathBuf,
    /// The LUN the image starts at sector 0 of.
    pub lun: u8,
}

/// How long one image is, checked against the room it is written into.
///
/// An image may be shorter than its partition — a partial backup is restored as
/// far as it goes — but one byte longer is refused: the extra sectors would
/// spill into whatever the phone keeps after it. `what` names the target in the
/// error, whichever kind of partition it is.
fn image_length(path: &Path, capacity: u64, what: &str) -> Result<u64, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?
        .len();
    if length == 0 {
        return Err(format!("{} is empty", path.display()));
    }
    if length > capacity {
        return Err(format!(
            "{} holds {length} bytes, but {what} only has {capacity} bytes",
            path.display()
        ));
    }

    Ok(length)
}

/// How long one image is, checked against the LUN it is written to.
fn checked_image(image: &LunImage, luns: &[Lun]) -> Result<(Lun, u64), String> {
    let lun = luns
        .iter()
        .find(|lun| lun.number == image.lun)
        .ok_or_else(|| format!("this storage has no LUN {}", image.lun))?;

    let length = image_length(
        &image.path,
        lun.bytes(),
        &format!("LUN {}", lun.number),
    )?;

    Ok((lun.clone(), length))
}

/// Writes whole-storage images back to the physical partitions they belong to.
///
/// Nothing is erased first: a `<program>` writes the sectors the image covers,
/// so an image shorter than its LUN leaves the rest of it as it was. Every
/// size is checked before the first sector is written — a phone that was half
/// written is worse than one that was not touched at all.
pub fn flash_storage(
    port: &str,
    luns: &[Lun],
    images: &[LunImage],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!(
        "action: write {} image(s) to the whole storage",
        images.len()
    ));
    for image in images {
        log.line(format!("  lun {}: {}", image.lun, image.path.display()));
    }

    let result = flash_storage_session(port, luns, images, log, on_progress);

    match &result {
        Ok(count) => log.line(format!("done: {count} image(s) written")),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn flash_storage_session(
    port: &str,
    luns: &[Lun],
    images: &[LunImage],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    if images.is_empty() {
        return Err("there is no image to write".to_owned());
    }

    // Every image is checked against its LUN before the port is even opened.
    let mut checked = Vec::with_capacity(images.len());
    for image in images {
        let (lun, length) = checked_image(image, luns)?;
        checked.push((lun, length));
    }

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let overall_total = checked
        .iter()
        .map(|(lun, length)| padded(*length, lun.sector_size))
        .fold(0_u64, u64::saturating_add);
    let mut before = 0_u64;

    for (image, (lun, length)) in images.iter().zip(&checked) {
        let mut file = File::open(&image.path)
            .map_err(|error| format!("could not read {}: {error}", image.path.display()))?;

        session.line(format!(
            "writing {} ({length} bytes) to lun {} ({} bytes)",
            image.path.display(),
            lun.number,
            lun.bytes()
        ));
        program_image(
            &mut session,
            &lun.region(),
            &mut Image::File {
                file: &mut file,
                path: &image.path,
                length: *length,
            },
            before,
            overall_total,
            on_progress,
        )?;

        before = before.saturating_add(padded(*length, lun.sector_size));
    }

    Ok(images.len())
}

/// One partition image and the partition it is written back to.
///
/// The range is the one the table names, so the image lands exactly where the
/// partition is — and an image that is too long is refused rather than written
/// over the next one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionImage {
    pub path: PathBuf,
    pub region: Region,
}

/// Writes partition images back to the partitions they were dumped from.
///
/// This is the counterpart of a dump of selected partitions: one file per
/// partition, written back to the range the table gives it. Nothing is erased
/// first, so an image shorter than its partition leaves the rest of it as it
/// was, and every size is checked before the first sector is written.
pub fn flash_partitions(
    port: &str,
    images: &[RegionImage],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!("action: write {} partition image(s)", images.len()));
    for image in images {
        log.line(format!(
            "  lun {} {} (sectors {}..{}): {}",
            image.region.lun,
            image.region.name,
            image.region.start_lba,
            image.region.end_lba,
            image.path.display()
        ));
    }

    let result = flash_partitions_session(port, images, log, on_progress);

    match &result {
        Ok(count) => log.line(format!("done: {count} image(s) written")),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn flash_partitions_session(
    port: &str,
    images: &[RegionImage],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    if images.is_empty() {
        return Err("there is no image to write".to_owned());
    }

    // Every image is checked against its partition before the port is opened.
    let mut checked = Vec::with_capacity(images.len());
    for image in images {
        let length = image_length(
            &image.path,
            image.region.bytes(),
            &format!("partition {}", image.region.name),
        )?;
        checked.push(length);
    }

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let overall_total = images
        .iter()
        .zip(&checked)
        .map(|(image, length)| padded(*length, image.region.sector_size))
        .fold(0_u64, u64::saturating_add);
    let mut before = 0_u64;

    for (image, length) in images.iter().zip(&checked) {
        let mut file = File::open(&image.path)
            .map_err(|error| format!("could not read {}: {error}", image.path.display()))?;

        session.line(format!(
            "writing {} ({length} bytes) to lun {} {} (sectors {}..{})",
            image.path.display(),
            image.region.lun,
            image.region.name,
            image.region.start_lba,
            image.region.end_lba
        ));
        program_image(
            &mut session,
            &image.region,
            &mut Image::File {
                file: &mut file,
                path: &image.path,
                length: *length,
            },
            before,
            overall_total,
            on_progress,
        )?;

        before = before.saturating_add(padded(*length, image.region.sector_size));
    }

    Ok(images.len())
}

fn write_region(
    session: &mut Session<'_>,
    region: &Region,
    path: &Path,
    before: u64,
    overall_total: u64,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let mut file = File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;

    let total = region.bytes();
    let mut done = 0_u64;
    let mut first = region.start_lba;
    // One read may not ask for more than one payload, so a large partition is
    // read in as many commands as it takes.
    let chunk_sectors = (MAX_PAYLOAD_BYTES / region.sector_size as usize).max(1) as u64;
    while first <= region.end_lba {
        let count = (region.end_lba - first + 1).min(chunk_sectors);
        let data = read_sectors(session, region.lun, region.sector_size, first, count)?;

        file.write_all(&data)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;

        done = done.saturating_add(data.len() as u64);
        on_progress(Progress {
            name: region.name.clone(),
            done,
            total,
            overall_done: before.saturating_add(done),
            overall_total,
        });
        first += count;
    }

    file.flush()
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// Writes `image` into `region`, sector by sector.
///
/// The programmer is already configured for the storage the table was read
/// from, so this only reopens the port and sends one `<program>` command with
/// the image's data behind it. An image smaller than the partition is padded
/// with zeros to the next whole sector (which is what the reference tools do).
pub fn write_partition(
    port: &str,
    region: &Region,
    image: &Path,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!(
        "action: write {} to {} (lun {}, sectors {}..{})",
        image.display(),
        region.name,
        region.lun,
        region.start_lba,
        region.end_lba
    ));

    let result = write_session(port, region, image, log, on_progress);

    match &result {
        Ok(()) => log.line("done: the image was written"),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn write_session(
    port: &str,
    region: &Region,
    image: &Path,
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let size = std::fs::metadata(image)
        .map_err(|error| format!("could not read {}: {error}", image.display()))?
        .len();
    if size == 0 {
        return Err(format!("{} is empty", image.display()));
    }
    if size > region.bytes() {
        return Err(format!(
            "{} holds {size} bytes, but {} only has {} bytes",
            image.display(),
            region.name,
            region.bytes()
        ));
    }

    let mut file = File::open(image)
        .map_err(|error| format!("could not read {}: {error}", image.display()))?;

    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let total = padded(size, region.sector_size);
    program_image(
        &mut session,
        region,
        &mut Image::File { file: &mut file, path: image, length: size },
        0,
        total,
        on_progress,
    )
}

/// Where one `<program>` takes its bytes from.
enum Image<'a> {
    /// An image on disk, read as it is sent, so a large one never has to fit in
    /// memory.
    File {
        file: &'a mut File,
        path: &'a Path,
        length: u64,
    },
    /// An image held in memory.
    Bytes(&'a [u8]),
    /// Zeroes over the whole region, with `last` in its final byte.
    Blank { last: u8 },
}

impl Image<'_> {
    /// How many bytes of the region the image covers.
    fn length(&self, region: &Region) -> u64 {
        match self {
            Image::File { length, .. } => *length,
            Image::Bytes(data) => data.len() as u64,
            Image::Blank { .. } => region.bytes(),
        }
    }

    /// Fills `buffer` with the bytes at `offset`; whatever the image does not
    /// reach stays zero, which pads the last packet to whole sectors.
    fn fill(
        &mut self,
        region: &Region,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<(), String> {
        match self {
            Image::File { file, path, .. } => {
                let mut filled = 0_usize;
                while filled < buffer.len() {
                    let read = file
                        .read(&mut buffer[filled..])
                        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }

                Ok(())
            }
            Image::Bytes(data) => {
                let start = offset as usize;
                if start < data.len() {
                    let end = (start + buffer.len()).min(data.len());
                    buffer[..end - start].copy_from_slice(&data[start..end]);
                }

                Ok(())
            }
            Image::Blank { last } => {
                // The buffer arrives zeroed; only the very last byte of the
                // region carries the flag.
                if offset + buffer.len() as u64 == region.bytes() {
                    if let Some(byte) = buffer.last_mut() {
                        *byte = *last;
                    }
                }

                Ok(())
            }
        }
    }
}

/// Sends one `<program>` for `image` and streams its bytes behind it.
///
/// One packet may not carry more than the payload the programmer was configured
/// with, so a large image is written in as many packets as it takes.
fn program_image(
    session: &mut Session<'_>,
    region: &Region,
    image: &mut Image<'_>,
    before: u64,
    overall_total: u64,
    on_progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let length = image.length(region);
    if length == 0 {
        return Err(format!("there is nothing to write to {}", region.name));
    }
    if length > region.bytes() {
        return Err(format!(
            "the image holds {length} bytes, but {} only has {} bytes",
            region.name,
            region.bytes()
        ));
    }

    let sector_size = region.sector_size;
    let sectors = length.div_ceil(u64::from(sector_size));
    let xml = format!(
        "<data><program SECTOR_SIZE_IN_BYTES=\"{sector_size}\" \
num_partition_sectors=\"{sectors}\" slot=\"0\" physical_partition_number=\"{}\" \
start_sector=\"{}\" read_back_verify=\"0\" /></data>",
        region.lun, region.start_lba
    );

    let response = request(session, &xml)?;
    if !response.is_ack() {
        return Err(refusal(&response, "start writing the partition"));
    }

    let sectors_per_chunk = (MAX_PAYLOAD_BYTES / sector_size as usize).max(1) as u64;
    let total = sectors.saturating_mul(u64::from(sector_size));
    let mut left = sectors;
    let mut done = 0_u64;
    let mut offset = 0_u64;

    while left > 0 {
        let count = left.min(sectors_per_chunk);
        let chunk = count as usize * sector_size as usize;

        let mut buffer = vec![0_u8; chunk];
        image.fill(region, offset, &mut buffer)?;

        session.line(format!(">> {chunk} bytes of image data"));
        session.send(&buffer)?;

        left -= count;
        offset = offset.saturating_add(chunk as u64);
        done = done.saturating_add(chunk as u64);
        on_progress(Progress {
            name: region.name.clone(),
            done,
            total,
            overall_done: before.saturating_add(done),
            overall_total: overall_total.max(total),
        });
    }

    let response = read_response(session)?;
    if !response.is_ack() {
        return Err(refusal(&response, "finish writing the partition"));
    }

    Ok(())
}

/// One `<erase>` for a whole partition.
///
/// Firehose erases a sector range, so the command is the one `<read>` and
/// `<program>` already use: the sectors around the range are left alone.
fn erase_xml(region: &Region) -> String {
    format!(
        "<data><erase SECTOR_SIZE_IN_BYTES=\"{}\" num_partition_sectors=\"{}\" slot=\"0\" \
physical_partition_number=\"{}\" start_sector=\"{}\" /></data>",
        region.sector_size,
        region.sectors(),
        region.lun,
        region.start_lba
    )
}

/// Erases the sectors of every region, one ACKed command each.
///
/// The data is not overwritten with zeros — the storage's own erase does the
/// work — so this is quick, and the progress jumps from one partition to the
/// next.
pub fn erase(
    port: &str,
    regions: &[Region],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!("action: erase {} partition(s)", regions.len()));
    for region in regions {
        log.line(format!(
            "  {} (lun {}, sectors {}..{}, {} bytes)",
            region.name,
            region.lun,
            region.start_lba,
            region.end_lba,
            region.bytes()
        ));
    }

    let result = erase_session(port, regions, log, on_progress);

    match &result {
        Ok(count) => log.line(format!("done: {count} partition(s) erased")),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn erase_session(
    port: &str,
    regions: &[Region],
    log: &mut FlashLog,
    on_progress: &dyn Fn(Progress),
) -> Result<usize, String> {
    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    let overall_total = total_bytes(regions);
    let mut before = 0_u64;
    for region in regions {
        let xml = erase_xml(region);
        session.line(format!(
            "erasing {} (lun {}, sectors {}..{})",
            region.name, region.lun, region.start_lba, region.end_lba
        ));

        let response = request(&mut session, &xml)?;
        if !response.is_ack() {
            return Err(format!(
                "{}: {}",
                region.name,
                refusal(&response, "erase the partition")
            ));
        }

        before = before.saturating_add(region.bytes());
        on_progress(Progress {
            name: region.name.clone(),
            done: region.bytes(),
            total: region.bytes(),
            overall_done: before,
            overall_total,
        });
    }

    Ok(regions.len())
}

/// Reads the partition table again, for a storage the programmer already
/// knows.
///
/// The programmer still runs in the phone, so only the `<configure>` and the
/// GPT reads are repeated: no Sahara upload, and no table left over from an
/// earlier read.
pub fn refresh(port: &str, storage: StorageType, log: &mut FlashLog) -> Result<Table, String> {
    log.line(format!(
        "LMN Flash {} - Partition Manager",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(format!("port: {port}"));
    log.line(format!(
        "action: read the {} partition table again",
        storage_name(storage)
    ));

    let result = refresh_session(port, storage, log);

    match &result {
        Ok(table) => log.line(format!("done: {} partition(s)", table.partitions.len())),
        Err(error) => log.line(format!("failed: {error}")),
    }

    result
}

fn refresh_session(port: &str, storage: StorageType, log: &mut FlashLog) -> Result<Table, String> {
    let mut session = Session {
        transport: Transport::open(port)?,
        log,
    };
    session.set_read_timeout(FIREHOSE_TIMEOUT)?;

    configure(&mut session, storage)?;
    let partitions = read_storage_partitions(&mut session, storage, sector_size(storage))?;

    Ok(Table { storage, partitions })
}

/// `<name>.img` in `directory`, with a number appended when that name is
/// taken — a package can carry the same partition name on more than one LUN.
fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let base = crate::flash_log::file_name_part(name);
    let mut candidate = directory.join(format!("{base}.img"));
    let mut number = 2_u32;

    while candidate.exists() {
        candidate = directory.join(format!("{base} ({number}).img"));
        number += 1;
    }

    candidate
}

// --- Sahara ---------------------------------------------------------------

/// Reads the programmer file into the Sahara image slots the phone asks for.
///
/// A plain MBN/ELF is one image; a *melf* is a CPIO `newc` archive of several
/// images, whose entry names carry the image id (`13:prog_firehose_ddr.elf`).
fn programmer_images(path: &Path) -> Result<Vec<Option<Vec<u8>>>, String> {
    let blob = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    if blob.len() >= CPIO_MAGIC.len() && &blob[..CPIO_MAGIC.len()] == CPIO_MAGIC {
        decode_archive(&blob)
    } else {
        Ok(vec![Some(blob)])
    }
}

/// Splits a CPIO `newc` archive into its Sahara images, indexed by id.
fn decode_archive(blob: &[u8]) -> Result<Vec<Option<Vec<u8>>>, String> {
    let mut images: Vec<Option<Vec<u8>>> = Vec::new();
    let mut at = 0_usize;

    loop {
        if at + CPIO_HEADER_LEN > blob.len() {
            return Err("the programmer archive ends in the middle of an entry".to_owned());
        }
        let header = &blob[at..at + CPIO_HEADER_LEN];
        if &header[..CPIO_MAGIC.len()] != CPIO_MAGIC {
            return Err("the programmer archive holds a broken entry".to_owned());
        }

        // The fields are 8-digit ASCII hex numbers, in this order: ino, mode,
        // uid, gid, nlink, mtime, filesize, devmajor, devminor, rdevmajor,
        // rdevminor, namesize, check.
        let filesize = hex_field(&header[54..62])?;
        let namesize = hex_field(&header[94..102])?;

        let name_start = at + CPIO_HEADER_LEN;
        let name_end = name_start + namesize;
        if name_end > blob.len() {
            return Err("the programmer archive ends in the middle of a name".to_owned());
        }
        let name = String::from_utf8_lossy(&blob[name_start..name_end]);
        let name = name.trim_end_matches('\0');

        if name == CPIO_TRAILER {
            return Ok(images);
        }

        let id: u32 = name
            .split(':')
            .next()
            .unwrap_or_default()
            .parse()
            .map_err(|_| format!("the programmer archive names an image `{name}`, not `<id>:<name>`"))?;
        if id == 0 {
            return Err(format!("the programmer archive names an invalid image id in `{name}`"));
        }

        let data_start = align4(name_end);
        let data_end = data_start + filesize;
        if data_end > blob.len() {
            return Err("the programmer archive ends in the middle of an image".to_owned());
        }

        if id as usize >= images.len() {
            images.resize(id as usize + 1, None);
        }
        images[id as usize] = Some(blob[data_start..data_end].to_vec());

        at = align4(data_end);
    }
}

/// Reads one 8-digit ASCII hex field of a CPIO header.
fn hex_field(field: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(field)
        .map_err(|_| "the programmer archive holds a malformed entry".to_owned())?;

    usize::from_str_radix(text.trim(), 16)
        .map_err(|_| "the programmer archive holds a malformed entry".to_owned())
}

/// A CPIO entry's name and data are padded to a four-byte boundary.
fn align4(value: usize) -> usize {
    (value + 3) & !3
}

/// The transport with Sahara: answers the greeting, serves the image slices
/// the phone asks for, and finishes when the phone has every image.
///
/// Returns whether the upload went ahead: the boot ROM is asked who the phone
/// is before the programmer goes in, `confirm` decides whether it may, and a
/// `false` leaves the phone reset and nothing uploaded.
fn sahara_upload(
    session: &mut Session<'_>,
    programmer: &mut Programmer,
    confirm: &dyn Fn(Preflight) -> InjectAnswer,
) -> Result<bool, String> {
    let mut greeted = false;
    // A packet the exchange already read, waiting to be handled by the loop.
    let mut pending: Option<Sahara> = None;

    loop {
        let packet = match pending.take() {
            Some(packet) => packet,
            None => session.read_sahara()?,
        };

        match packet {
            Sahara::AlreadyFirehose => {
                session.line("sahara: the phone is already in Firehose mode");
                return Ok(true);
            }
            Sahara::Hello { version } => {
                session.line(format!("sahara: hello (protocol {version})"));

                // A boot ROM is the only thing that knows which SoC, OEM and
                // model the phone really is; a programmer is built for one of
                // each. The greeting is answered once, asking for command mode,
                // which is how the identity is read — and a boot ROM that has
                // taken commands only goes back to uploading on a mode switch.
                // A programmer that is several images long greets a second time
                // between them, and is not asked again.
                if !greeted {
                    greeted = true;

                    if version < 2 {
                        session.line(format!(
                            "sahara: protocol {version} has no command mode; the identity is not available"
                        ));
                        session.send_hello(SAHARA_MODE_IMAGE_TX_PENDING)?;
                        session.line("sahara: hello response (image transfer mode)");
                        continue;
                    }

                    if let Err(error) = session.enter_command_mode() {
                        session.line(format!(
                            "sahara: the phone did not take commands ({error}); the upload goes on without an identity"
                        ));
                        session.send_hello(SAHARA_MODE_IMAGE_TX_PENDING)?;
                        session.line("sahara: hello response (image transfer mode)");
                        continue;
                    }

                    match read_identity(session, version) {
                        Ok(identity) => {
                            session.line(identity.log_line());

                            // The card stays up until the user goes on: another
                            // file may be picked in the meantime, and nothing has
                            // been uploaded yet, so the session simply goes on
                            // with it.
                            loop {
                                let preflight = Preflight {
                                    identity: identity.clone(),
                                    programmer: programmer.path.clone(),
                                    programmer_hash: programmer_root_hash(
                                        &programmer.images,
                                        crate::mbn::Hash::of_length(
                                            identity.pk_hash.as_ref().map_or(0, Vec::len),
                                        ),
                                    ),
                                };

                                match &preflight.programmer_hash {
                                    Ok(hash) => session.line(format!(
                                        "programmer: root key hash {} ({} bytes)",
                                        hash.iter()
                                            .map(|byte| format!("{byte:02X}"))
                                            .collect::<String>(),
                                        hash.len()
                                    )),
                                    Err(error) => session.line(format!(
                                        "programmer: the root key hash was not read ({error})"
                                    )),
                                }

                                match confirm(preflight) {
                                    InjectAnswer::Upload => break,
                                    InjectAnswer::Cancel => {
                                        session.line("sahara: the user stopped the upload");
                                        // The phone is still taking commands, and
                                        // the reset is the one packet that ends a
                                        // session from anywhere. If it is not
                                        // answered either, the log says so — the
                                        // page still reports a cancellation, which
                                        // is what it was.
                                        match session.sahara_reset() {
                                            Ok(()) => session.line("sahara: the phone was reset"),
                                            Err(error) => session.line(format!(
                                                "sahara: the reset was not answered ({error}); the phone may have to be unplugged"
                                            )),
                                        }
                                        return Ok(false);
                                    }
                                    InjectAnswer::Reselect(path) => {
                                        session.line(format!(
                                            "sahara: another programmer was picked: {}",
                                            path.display()
                                        ));

                                        match Programmer::open(&path) {
                                            Ok(picked) => {
                                                session.line(picked.log_line());
                                                *programmer = picked;
                                            }
                                            // The card comes back with the file
                                            // that is still loaded, and the log
                                            // says why the new one was not.
                                            Err(error) => session
                                                .line(format!("programmer: {error}")),
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => session.line(format!(
                            "sahara: the identity was not read ({error}); the upload goes on with it unknown"
                        )),
                    }

                    // Whether or not the phone answered, it is taking commands
                    // now, where a greeting is refused: the mode switch is what
                    // puts it back in front of the upload. What it answers to
                    // that is the boot ROM's business — being ready, asking for
                    // the first slice, or nothing at all — so the answer is put
                    // back for the loop to handle, and a boot ROM that says
                    // nothing gets the greeting it is waiting for.
                    session.switch_to_upload_mode()?;
                    match session.try_read_sahara(MODE_SWITCH_WAIT)? {
                        Some(answer) => {
                            session.line("sahara: switched back to image transfer mode");
                            pending = Some(answer);
                        }
                        None => {
                            session.line(
                                "sahara: the mode switch was not answered; greeting the phone instead",
                            );
                            session.send_hello(SAHARA_MODE_IMAGE_TX_PENDING)?;
                            session.line("sahara: hello response (image transfer mode)");
                        }
                    }
                    continue;
                }

                session.send_hello(SAHARA_MODE_IMAGE_TX_PENDING)?;
                session.line("sahara: hello response (image transfer mode)");
            }
            Sahara::ReadData { image, offset, len } => {
                if !greeted {
                    return Err("the phone asked for an image before it was greeted".to_owned());
                }
                session.line(format!(
                    "sahara: image {image}, {} bytes at offset {offset}",
                    len
                ));
                // A single-image programmer is always added as image 0, even
                // when the phone asks for id 13.
                let index = if programmer.images.len() == 1 { 0 } else { image as usize };
                let Some(Some(data)) = programmer.images.get(index) else {
                    return Err(format!("the phone asked for image {image}, which the programmer does not carry"));
                };

                let start = offset as usize;
                let end = start.saturating_add(len as usize);
                if end > data.len() {
                    return Err(format!(
                        "the phone asked for bytes {start}..{end} of image {image}, which is {} bytes long",
                        data.len()
                    ));
                }
                session.send(&data[start..end])?;
            }
            Sahara::EndOfImage { status } => {
                session.line(format!("sahara: end of image (status {status})"));
                if status != 0 {
                    return Err(format!("the phone rejected the programmer (Sahara status {status})"));
                }
                session.send_packet(SAHARA_DONE, &[])?;
            }
            Sahara::DoneResp { status } => {
                session.line(format!("sahara: done response (status {status})"));
                // 1 means every image went through; a multi-image programmer
                // keeps the loop alive while the phone asks for the next one.
                if status == 1 || programmer.images.len() <= 1 {
                    return Ok(true);
                }
            }
            // The boot ROM said it was ready for the upload after the mode
            // switch; the first slice request follows.
            Sahara::CommandReady => session.line("sahara: ready for the image"),
            Sahara::CommandResponse { .. } | Sahara::Reset => {
                return Err(
                    "the phone answered a command while the programmer was being uploaded"
                        .to_owned(),
                );
            }
        }
    }
}

/// Reads the answers the boot ROM gives about itself, once it is taking
/// commands.
///
/// Two commands: the hardware ID, and then the OEM public key hash. A boot ROM
/// that keeps the hash back is no reason to stop — the ids are what a programmer
/// is matched by, and a protocol 3 boot ROM is known to withhold the hash — so
/// only the hardware ID read can fail the lot.
fn read_identity(session: &mut Session<'_>, version: u32) -> Result<Identity, String> {
    let hw_id = session.command(SAHARA_READ_HW_ID)?;
    if hw_id.len() < 8 {
        return Err(format!(
            "the phone answered {} bytes to the hardware ID read, not eight",
            hw_id.len()
        ));
    }

    let mut identity = Identity::of(u64::from_le_bytes(hw_id[..8].try_into().unwrap()));

    if version < 3 {
        match session.command(SAHARA_READ_PK_HASH) {
            Ok(hash) if hash.len() >= 32 => identity.pk_hash = Some(hash[..32].to_vec()),
            Ok(hash) => session.line(format!(
                "sahara: the PK hash read answered {} bytes, not the 32 of a SHA-256",
                hash.len()
            )),
            Err(error) => session.line(format!("sahara: the PK hash was not read ({error})")),
        }
    } else {
        session.line(format!(
            "sahara: protocol {version} does not hand out the PK hash"
        ));
    }

    Ok(identity)
}

/// Reads the hash of the root certificate a programmer is signed against.
///
/// A `.melf` carries several images and only some of them are signed, so every
/// one of them is tried until one holds a chain.
fn programmer_root_hash(
    images: &[Option<Vec<u8>>],
    hash: crate::mbn::Hash,
) -> Result<Vec<u8>, String> {
    let mut images = images.iter().flatten();
    let mut reason = String::from("the programmer carries no image");

    for image in images.by_ref() {
        match crate::mbn::root_hash(image, hash) {
            Ok(root) => return Ok(root),
            Err(error) => reason = error,
        }
    }

    Err(reason)
}

/// The Sahara packets the upload reacts to.
enum Sahara {
    Hello {
        /// The boot ROM's Sahara version: a protocol 2 boot ROM is the one
        /// that can be asked who the phone is.
        version: u32,
    },
    ReadData {
        image: u64,
        offset: u64,
        len: u64,
    },
    EndOfImage {
        status: u32,
    },
    DoneResp {
        status: u32,
    },
    AlreadyFirehose,
    /// The boot ROM took the mode it was asked for and waits for commands.
    CommandReady,
    /// The boot ROM ran a command and has `len` bytes ready to be read.
    CommandResponse { command: u32, len: u32 },
    /// The boot ROM is resetting.
    Reset,
}

impl Sahara {
    /// What the packet was, for the errors that name one.
    fn name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "a greeting",
            Self::ReadData { .. } => "an image request",
            Self::EndOfImage { .. } => "an end of image",
            Self::DoneResp { .. } => "a done response",
            Self::AlreadyFirehose => "Firehose XML",
            Self::CommandReady => "a command-ready answer",
            Self::CommandResponse { .. } => "a command response",
            Self::Reset => "a reset response",
        }
    }
}

// --- Firehose -------------------------------------------------------------

/// Configures the phone for one storage media.
fn configure(session: &mut Session<'_>, storage: StorageType) -> Result<(), String> {
    let name = memory_name(storage).unwrap_or("emmc");
    let xml = format!(
        "<data><configure MemoryName=\"{name}\" Verbose=\"0\" AlwaysValidate=\"0\" \
MaxDigestTableSizeInBytes=\"8192\" MaxPayloadSizeToTargetInBytes=\"{MAX_PAYLOAD_BYTES}\" \
ZlpAwareHost=\"1\" SkipStorageInit=\"0\" /></data>"
    );

    let response = request(session, &xml)?;
    if response.is_ack() {
        Ok(())
    } else {
        Err(refusal(&response, "configure the storage"))
    }
}

/// Reads the partition table of every physical partition the media offers.
fn read_storage_partitions(
    session: &mut Session<'_>,
    storage: StorageType,
    sector_size: u32,
) -> Result<Vec<Partition>, String> {
    // Only UFS has several LUNs; every other media is one GPT.
    if storage != StorageType::Ufs {
        return read_gpt(session, 0, sector_size);
    }

    let mut partitions = Vec::new();
    for lun in 0..MAX_LUNS {
        match read_gpt(session, lun, sector_size) {
            Ok(mut found) => partitions.append(&mut found),
            Err(error) => {
                // LUN 0 has to work; the first LUN that does not answer is
                // simply the one the phone does not have, which ends the scan.
                if lun == 0 {
                    session.line(format!("lun 0: reading the table failed: {error}"));
                    return Err(error);
                }
                session.line(format!("lun {lun}: not available ({error}); the scan ends here"));
                break;
            }
        }
    }

    Ok(partitions)
}

/// Reads and parses the GPT of one LUN.
fn read_gpt(
    session: &mut Session<'_>,
    lun: u8,
    sector_size: u32,
) -> Result<Vec<Partition>, String> {
    // Sector 0 is the protective MBR, so the header sits in sector 1.
    let header = read_sectors(session, lun, sector_size, 1, 1)?;
    let header = parse_gpt_header(&header)?;

    let table_bytes = u64::from(header.entry_count) * u64::from(header.entry_size);
    let table_sectors = table_bytes.div_ceil(u64::from(sector_size));
    let table = read_sectors(
        session,
        lun,
        sector_size,
        header.entries_lba,
        table_sectors,
    )?;

    let partitions = parse_gpt_entries(&table, lun, sector_size, &header)?;
    session.line(format!(
        "lun {lun}: GPT with {} entries, {} in use",
        header.entry_count,
        partitions.len()
    ));

    Ok(partitions)
}

/// The header fields a GPT read needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GptHeader {
    entries_lba: u64,
    entry_count: u32,
    entry_size: u32,
}

fn parse_gpt_header(sector: &[u8]) -> Result<GptHeader, String> {
    const REQUIRED: usize = 92;

    if sector.len() < REQUIRED || &sector[..8] != b"EFI PART" {
        return Err("no GUID partition table".to_owned());
    }

    let entries_lba = u64::from_le_bytes(sector[72..80].try_into().unwrap());
    let entry_count = u32::from_le_bytes(sector[80..84].try_into().unwrap());
    let entry_size = u32::from_le_bytes(sector[84..88].try_into().unwrap());

    // 128 bytes is the size the specification defines; bigger entries are
    // allowed, and a table with no entries is not a usable one.
    if !(128..=4096).contains(&entry_size) || entry_count == 0 || entry_count > 4096 {
        return Err("the partition table header is not usable".to_owned());
    }

    Ok(GptHeader {
        entries_lba,
        entry_count,
        entry_size,
    })
}

fn parse_gpt_entries(
    table: &[u8],
    lun: u8,
    sector_size: u32,
    header: &GptHeader,
) -> Result<Vec<Partition>, String> {
    let entry_size = header.entry_size as usize;
    let needed = header.entry_count as usize * entry_size;
    if table.len() < needed {
        return Err("the partition table is shorter than its header says".to_owned());
    }

    let mut partitions = Vec::new();
    for index in 0..header.entry_count as usize {
        let entry = &table[index * entry_size..(index + 1) * entry_size];

        // An unused entry has a zeroed partition type GUID.
        if entry[..16].iter().all(|byte| *byte == 0) {
            continue;
        }

        let start_lba = u64::from_le_bytes(entry[32..40].try_into().unwrap());
        let end_lba = u64::from_le_bytes(entry[40..48].try_into().unwrap());

        partitions.push(Partition {
            name: decode_utf16(&entry[56..entry_size.min(128)]),
            lun,
            start_lba,
            end_lba,
            sector_size,
        });
    }

    Ok(partitions)
}

/// A GPT name is UTF-16LE, padded with NULs to 72 bytes.
fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();

    String::from_utf16_lossy(&units)
}

/// Asks one physical partition how many sectors it has.
///
/// `<getstorageinfo>` is the only command that names the size of a LUN the way
/// the phone sees it, including the ones a partition table is not in: UFS
/// keeps user data on a LUN of its own, with no GPT to read.
fn storage_info(session: &mut Session<'_>, lun: u8, sector_size: u32) -> Result<u64, String> {
    let xml = format!("<data><getstorageinfo physical_partition_number=\"{lun}\" /></data>");

    let response = request(session, &xml)?;
    if !response.is_ack() {
        return Err(refusal(&response, "read the storage size"));
    }

    reported_sectors(&response, sector_size)
        .ok_or_else(|| "the phone did not say how large the partition is".to_owned())
}

/// How many sectors a `<getstorageinfo>` reply says one LUN has.
///
/// Programmers spell the attribute differently (`TOTAL_SECTORS` and
/// `total_sectors` are both in use), and a few answer in kilobytes instead of
/// sectors, so both are read.
fn reported_sectors(response: &Response, sector_size: u32) -> Option<u64> {
    if let Some(sectors) = response
        .attribute("TOTAL_SECTORS")
        .and_then(|text| text.trim().parse::<u64>().ok())
    {
        return Some(sectors);
    }

    response
        .attribute("SIZE_IN_KBYTES")
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map(|kilobytes| kilobytes.saturating_mul(1024) / u64::from(sector_size))
}

/// The sectors the table says one LUN has: the end of its last partition.
fn table_extent(partitions: &[Partition], lun: u8) -> Option<u64> {
    partitions
        .iter()
        .filter(|partition| partition.lun == lun)
        .map(|partition| partition.end_lba.saturating_add(1))
        .max()
}

/// Reads `count` sectors of one LUN and returns their bytes.
fn read_sectors(
    session: &mut Session<'_>,
    lun: u8,
    sector_size: u32,
    first: u64,
    count: u64,
) -> Result<Vec<u8>, String> {
    let length = count as usize * sector_size as usize;
    if length > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "one read of {length} bytes is more than the {MAX_PAYLOAD_BYTES}-byte payload"
        ));
    }

    let xml = format!(
        "<data><read SECTOR_SIZE_IN_BYTES=\"{sector_size}\" \
num_partition_sectors=\"{count}\" slot=\"0\" physical_partition_number=\"{lun}\" \
start_sector=\"{first}\" /></data>"
    );

    let response = request(session, &xml)?;
    if !response.is_ack() {
        return Err(refusal(&response, "read from the phone"));
    }

    let mut data = vec![0_u8; length];
    session.read_exact(&mut data).map_err(|error| {
        session.line(format!("note: reading {length} bytes of sector data failed: {error}"));
        error
    })?;
    session.line(format!("<< {length} bytes of sector data"));

    // The data is followed by the response that says it was sent in full.
    let response = read_response(session)?;
    if !response.is_ack() {
        return Err(refusal(&response, "read from the phone"));
    }

    Ok(data)
}

/// Sends one XML command and reads the response, without judging it.
fn request(session: &mut Session<'_>, xml: &str) -> Result<Response, String> {
    send_xml(session, xml)?;
    read_response(session)
}

/// The declaration every Firehose packet starts with.
///
/// It is not decoration: the bootloader in an 8998 phone answered a `<read>`
/// without it with `NAK` and `**WARNING Ignoring Unrecognized tag 'data'`,
/// while the same command with it works. Every packet carries it, so no
/// command can be forgotten.
const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>";

/// Sends an XML command the way Firehose expects it.
fn send_xml(session: &mut Session<'_>, xml: &str) -> Result<(), String> {
    let packet = packet(xml);

    session.line(format!(">> {packet}"));
    let result = session.send(&outgoing(&packet));
    if let Err(error) = &result {
        session.line(format!("!! {error}"));
    }

    result
}

/// One command with the declaration in front of it.
fn packet(xml: &str) -> String {
    if xml.starts_with("<?xml") {
        xml.to_owned()
    } else {
        format!("{XML_DECLARATION}{xml}")
    }
}

/// The bytes of one XML command.
///
/// A packet whose length is a multiple of 512 bytes would look padded to the
/// phone, so a newline is appended to it — exactly what the reference tools
/// do.
fn outgoing(xml: &str) -> Vec<u8> {
    let mut bytes = xml.as_bytes().to_vec();
    if !bytes.is_empty() && bytes.len() % 512 == 0 {
        bytes.push(b'\n');
    }

    bytes
}

/// One `<response>` element, with the `<log>` lines that came before it.
struct Response {
    value: String,
    logs: Vec<String>,
    /// Every `name="value"` the element carries. A few commands answer with
    /// what they were asked for — `<getstorageinfo>` reports the size of a
    /// LUN this way — and the names differ from programmer to programmer.
    attributes: Vec<(String, String)>,
}

impl Response {
    fn is_ack(&self) -> bool {
        self.value.eq_ignore_ascii_case("ACK")
    }

    /// The phone's own log lines, as one sentence.
    fn log_text(&self) -> String {
        self.logs.join(" ")
    }

    /// The value of one attribute, whatever case and underscores the
    /// programmer spelled it with: `TOTAL_SECTORS`, `total_sectors` and
    /// `TotalSectors` are the same name.
    fn attribute(&self, name: &str) -> Option<&str> {
        let wanted = normalized_name(name);

        self.attributes
            .iter()
            .find(|(key, _)| normalized_name(key) == wanted)
            .map(|(_, value)| value.as_str())
    }
}

/// A name with its case and underscores dropped, so attribute names can be
/// compared without depending on how a programmer spells them.
fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The error a NAK (or any other non-ACK answer) stands for.
fn refusal(response: &Response, what: &str) -> String {
    let value = if response.value.is_empty() {
        "no value"
    } else {
        response.value.as_str()
    };
    let log = response.log_text();

    if log.is_empty() {
        format!("could not {what}: the phone answered {value}")
    } else {
        format!("could not {what}: {value}: {log}")
    }
}

/// Reads until a whole `<response>` element has arrived.
fn read_response(session: &mut Session<'_>) -> Result<Response, String> {
    read_conversation(session).map(|(_, response)| response)
}

/// Reads one command's whole answer, as the phone sent it.
///
/// A programmer may report in a burst: one `<data>` element per `<log>` line,
/// and only the last of them carrying the `<response>`. The answer is over
/// when that element arrives, but everything before it is part of what the
/// phone said — a "Flash Info" panel is made of exactly those lines.
fn read_conversation(session: &mut Session<'_>) -> Result<(String, Response), String> {
    let mut buffer = Vec::new();
    let mut byte = [0_u8; 1];

    loop {
        session.read_exact(&mut byte).map_err(|error| {
            session.line(format!("!! {error}"));
            error
        })?;
        buffer.push(byte[0]);

        if buffer.len() > MAX_RESPONSE {
            return Err("the phone sent an answer too long to read".to_owned());
        }

        // Every element ends with `>`; checking only there keeps the scan
        // from running over the buffer for every byte.
        if byte[0] == b'>' {
            if let Some(response) = complete_response(&buffer) {
                let value = if response.value.is_empty() {
                    "?"
                } else {
                    response.value.as_str()
                };
                session.line(format!("<< <response value=\"{value}\">"));
                let logs = response.logs.clone();
                for line in logs {
                    session.line(format!("<< <log value=\"{line}\">"));
                }

                let conversation = String::from_utf8_lossy(&buffer).into_owned();
                return Ok((conversation, response));
            }
        }
    }
}

/// The response in `bytes`, when the whole reply has arrived.
///
/// A Firehose reply is one `<data>` element, and the raw storage data a
/// `<read>` answer carries starts only after its `</data>`. Stopping at the
/// `<response … />` instead would leave the closing tag in the stream, so the
/// first seven bytes of every sector read would be `</data>` — which is
/// exactly how a GPT read came back as "no GUID partition table".
fn complete_response(bytes: &[u8]) -> Option<Response> {
    let text = String::from_utf8_lossy(bytes);

    let start = text.rfind("<data")?;
    let closing = "</data>";
    let end = start + text[start..].find(closing)? + closing.len();
    let batch = &text[start..end];

    let response_at = batch.rfind("<response")?;
    let element = &batch[response_at..];
    if !element.contains("/>") && !element.contains("</response>") {
        return None;
    }

    let tag_end = element.find('>').map_or(element.len(), |end| end + 1);

    Some(Response {
        value: attributes(batch, "response", "value")
            .pop()
            .unwrap_or_default(),
        logs: attributes(batch, "log", "value"),
        attributes: element_attributes(&element[..tag_end]),
    })
}

/// Every `name="value"` attribute of one element.
fn element_attributes(element: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = element;

    while let Some(equals) = rest.find("=\"") {
        // The name is what directly precedes the `=`: the tag's own name or a
        // space separates it from the rest of the element.
        let name_start = rest[..equals]
            .rfind(|character: char| {
                character.is_whitespace() || character == '<' || character == '/'
            })
            .map_or(0, |index| index + 1);
        let name = &rest[name_start..equals];
        let value_start = equals + 2;
        let Some(end) = rest[value_start..].find('"') else {
            break;
        };

        if !name.is_empty() {
            pairs.push((
                name.to_owned(),
                unescape(&rest[value_start..value_start + end]),
            ));
        }

        rest = &rest[value_start + end + 1..];
    }

    pairs
}

/// Every `attr="…"` value of every `<tag …>` element in `text`.
fn attributes(text: &str, tag: &str, attr: &str) -> Vec<String> {
    let opening = format!("<{tag}");
    let key = format!("{attr}=\"");
    let mut values = Vec::new();
    let mut rest = text;

    while let Some(found) = rest.find(&opening) {
        let element = &rest[found..];
        let end = element.find('>').map_or(element.len(), |end| end + 1);
        let element = &element[..end];

        if let Some(at) = element.find(&key) {
            let value = &element[at + key.len()..];
            if let Some(end) = value.find('"') {
                values.push(unescape(&value[..end]));
            }
        }

        rest = &rest[found + opening.len()..];
    }

    values
}

/// Turns the five predefined XML entities back into their characters, so a
/// phone's log line is readable in the page's status line.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// --- Transport ------------------------------------------------------------

/// The serial port together with the session log.
///
/// Everything the protocol does goes through this: the port itself, and the
/// line-by-line account the user can send on when an injection fails.
struct Session<'a> {
    transport: Transport,
    log: &'a mut FlashLog,
}

impl Session<'_> {
    /// Writes one line to the session log.
    fn line(&mut self, text: impl AsRef<str>) {
        self.log.line(text.as_ref());
    }
}

impl std::ops::Deref for Session<'_> {
    type Target = Transport;

    fn deref(&self) -> &Transport {
        &self.transport
    }
}

impl std::ops::DerefMut for Session<'_> {
    fn deref_mut(&mut self) -> &mut Transport {
        &mut self.transport
    }
}

/// The serial port, with a pushback of the bytes that turned out to belong to
/// Firehose rather than to Sahara.
struct Transport {
    port: SerialPort,
    pending: Vec<u8>,
    started: usize,
    /// How long a read waits for the phone, so a shorter window can be tried
    /// and the port put back the way it was.
    timeout: Duration,
}

impl Transport {
    fn open(name: &str) -> Result<Self, String> {
        let mut port = SerialPort::open(name, |mut settings: serial2::Settings| {
            settings.set_raw();
            settings.set_baud_rate(BAUD_RATE)?;
            Ok(settings)
        })
        .map_err(|error| format!("could not open {name}: {error}"))?;

        port.set_read_timeout(TIMEOUT)
            .map_err(|error| format!("could not set the read timeout on {name}: {error}"))?;
        port.set_write_timeout(TIMEOUT)
            .map_err(|error| format!("could not set the write timeout on {name}: {error}"))?;

        Ok(Self {
            port,
            pending: Vec::new(),
            started: 0,
            timeout: TIMEOUT,
        })
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        // No `flush()` here: on Windows, `FlushFileBuffers` fails on a
        // communications handle with ERROR_INVALID_FUNCTION ("incorrect
        // function", os error 1) even though the bytes reached the driver.
        // Calling it made every write report a failure; `write_all` is the
        // whole job.
        self.port
            .write_all(bytes)
            .map_err(|error| format!("could not write to the phone: {error}"))
    }

    /// Changes how long a read waits for the phone: the boot ROM and the
    /// programmer it starts are two programs with different timing.
    fn set_read_timeout(&mut self, timeout: Duration) -> Result<(), String> {
        self.port
            .set_read_timeout(timeout)
            .map_err(|error| format!("could not change the read timeout: {error}"))?;
        self.timeout = timeout;

        Ok(())
    }

    /// Reads one Sahara packet, but gives up after `window` and answers `None`
    /// when the phone says nothing in it.
    ///
    /// What a boot ROM answers to a mode switch varies: a boot ROM that stays
    /// silent is not an error here, it is one that wants to be greeted instead —
    /// the caller decides, and the port is left waiting as long as it did.
    fn try_read_sahara(&mut self, window: Duration) -> Result<Option<Sahara>, String> {
        let previous = self.timeout;
        self.port
            .set_read_timeout(window)
            .map_err(|error| format!("could not change the read timeout: {error}"))?;

        let answer = self.read_sahara();

        self.port
            .set_read_timeout(previous)
            .map_err(|error| format!("could not change the read timeout: {error}"))?;

        match answer {
            Ok(packet) => Ok(Some(packet)),
            // Nothing arrived, or what arrived was not a packet: the exchange
            // below either greets the phone or ends with its own error.
            Err(_) => Ok(None),
        }
    }

    fn read_exact(&mut self, out: &mut [u8]) -> Result<(), String> {
        let mut filled = 0;

        if self.started < self.pending.len() {
            let available = &self.pending[self.started..];
            let take = available.len().min(out.len());
            out[..take].copy_from_slice(&available[..take]);
            self.started += take;
            filled = take;

            if self.started == self.pending.len() {
                self.pending.clear();
                self.started = 0;
            }
        }

        if filled < out.len() {
            self.port.read_exact(&mut out[filled..]).map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    format!(
                        "the phone did not answer within {} seconds",
                        TIMEOUT.as_secs()
                    )
                } else {
                    format!("could not read from the phone: {error}")
                }
            })?;
        }

        Ok(())
    }

    /// Puts bytes back, so the Firehose reader sees the XML that Sahara read
    /// the first eight bytes of.
    fn unread(&mut self, bytes: &[u8]) {
        if self.started < self.pending.len() {
            self.pending.drain(..self.started);
            self.started = 0;
        }

        let mut restored = bytes.to_vec();
        restored.extend_from_slice(&self.pending);
        self.pending = restored;
        self.started = 0;
    }

    fn send_packet(&mut self, command: u32, body: &[u8]) -> Result<(), String> {
        let mut packet = Vec::with_capacity(8 + body.len());
        packet.extend_from_slice(&command.to_le_bytes());
        packet.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        packet.extend_from_slice(body);

        self.send(&packet)
    }

    /// Answers the boot ROM's greeting, asking for `mode`.
    ///
    /// The version and "compatible" numbers are 2 and 1 like the reference
    /// tools report, and the status is success.
    fn send_hello(&mut self, mode: u32) -> Result<(), String> {
        let mut body = Vec::with_capacity(40);
        body.extend_from_slice(&SAHARA_VERSION.to_le_bytes());
        body.extend_from_slice(&1_u32.to_le_bytes());
        body.extend_from_slice(&0_u32.to_le_bytes());
        body.extend_from_slice(&mode.to_le_bytes());
        body.extend_from_slice(&[0_u8; 24]);

        self.send_packet(SAHARA_HELLO_RESP, &body)
    }

    /// Runs one of the boot ROM's own commands and reads its answer.
    ///
    /// The exchange is three packets and a raw read: the command goes out, the
    /// boot ROM says how many bytes it has, the host asks for them, and the
    /// bytes follow.
    fn command(&mut self, command: u32) -> Result<Vec<u8>, String> {
        let mut body = Vec::with_capacity(4);
        body.extend_from_slice(&command.to_le_bytes());
        self.send_packet(SAHARA_EXECUTE, &body)?;

        let len = match self.read_sahara()? {
            Sahara::CommandResponse {
                command: answered,
                len,
            } => {
                if answered != command {
                    return Err(format!(
                        "the phone answered command {answered:#x} when {command:#x} was asked"
                    ));
                }
                len
            }
            other => {
                return Err(format!(
                    "the phone answered {} instead of a command response",
                    other.name()
                ));
            }
        };

        self.send_packet(SAHARA_EXECUTE_DATA, &body)?;

        if len > SAHARA_MAX_PACKET {
            return Err(format!(
                "the phone has {len} bytes to read, more than a command may carry"
            ));
        }

        let mut data = vec![0_u8; len as usize];
        self.read_exact(&mut data)?;

        Ok(data)
    }

    /// Answers the greeting with a request for command mode, and waits for the
    /// boot ROM to say it is ready for commands.
    fn enter_command_mode(&mut self) -> Result<(), String> {
        self.send_hello(SAHARA_MODE_COMMAND)?;

        match self.read_sahara()? {
            Sahara::CommandReady => Ok(()),
            other => Err(format!(
                "the phone did not accept command mode ({})",
                other.name()
            )),
        }
    }

    /// Puts a boot ROM that is taking commands back into the mode it uploads
    /// in.
    ///
    /// A greeting is not accepted any more once the boot ROM is in command mode
    /// — it answers one with a NAK, which is where an injection used to die —
    /// so the only way back is Sahara's mode switch. What comes back is left to
    /// the upload loop: a boot ROM may say it is ready first, or start asking
    /// for the first slice straight away.
    fn switch_to_upload_mode(&mut self) -> Result<(), String> {
        self.send_packet(
            SAHARA_SWITCH_MODE,
            &SAHARA_MODE_IMAGE_TX_PENDING.to_le_bytes(),
        )
    }

    /// Asks the boot ROM to reset the phone — the way out of a session that
    /// will not upload anything.
    fn sahara_reset(&mut self) -> Result<(), String> {
        self.send_packet(SAHARA_RESET, &[])?;

        match self.read_sahara()? {
            Sahara::Reset => Ok(()),
            other => Err(format!(
                "the phone answered {} to the reset request",
                other.name()
            )),
        }
    }

    /// Reads one Sahara packet.
    fn read_sahara(&mut self) -> Result<Sahara, String> {
        let mut header = [0_u8; 8];
        self.read_exact(&mut header)?;

        let command = u32::from_le_bytes(header[..4].try_into().unwrap());
        let length = u32::from_le_bytes(header[4..].try_into().unwrap());

        if command == SAHARA_XML {
            self.unread(&header);
            return Ok(Sahara::AlreadyFirehose);
        }

        if !(8..=SAHARA_MAX_PACKET).contains(&length) {
            return Err(format!(
                "the phone sent a malformed Sahara packet (command {command:#x}, {length} bytes)"
            ));
        }

        let mut body = vec![0_u8; length as usize - 8];
        self.read_exact(&mut body)?;

        parse_sahara(command, &body)
    }
}

/// Reads the body of one Sahara packet.
fn parse_sahara(command: u32, body: &[u8]) -> Result<Sahara, String> {
    match command {
        SAHARA_HELLO => {
            // The greeting is twelve 32-bit fields: the command and its
            // length, the protocol version, the oldest one it accepts, the
            // largest packet it takes, the mode it is in, then six reserved.
            if body.len() < 40 {
                return Err("the phone sent a malformed greeting".to_owned());
            }

            Ok(Sahara::Hello {
                version: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            })
        }
        SAHARA_CMD_READY => Ok(Sahara::CommandReady),
        SAHARA_RESET_RESP => Ok(Sahara::Reset),
        SAHARA_EXECUTE_RESP => {
            if body.len() < 8 {
                return Err("the phone sent a malformed command response".to_owned());
            }
            Ok(Sahara::CommandResponse {
                command: u32::from_le_bytes(body[0..4].try_into().unwrap()),
                len: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            })
        }
        SAHARA_READ_DATA => {
            if body.len() < 12 {
                return Err("the phone sent a malformed image request".to_owned());
            }
            Ok(Sahara::ReadData {
                image: u64::from(u32::from_le_bytes(body[0..4].try_into().unwrap())),
                offset: u64::from(u32::from_le_bytes(body[4..8].try_into().unwrap())),
                len: u64::from(u32::from_le_bytes(body[8..12].try_into().unwrap())),
            })
        }
        SAHARA_READ_DATA_64 => {
            if body.len() < 24 {
                return Err("the phone sent a malformed 64-bit image request".to_owned());
            }
            Ok(Sahara::ReadData {
                image: u64::from_le_bytes(body[0..8].try_into().unwrap()),
                offset: u64::from_le_bytes(body[8..16].try_into().unwrap()),
                len: u64::from_le_bytes(body[16..24].try_into().unwrap()),
            })
        }
        SAHARA_END_OF_IMAGE => {
            if body.len() < 8 {
                return Err("the phone sent a malformed end-of-image packet".to_owned());
            }
            Ok(Sahara::EndOfImage {
                status: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            })
        }
        SAHARA_DONE_RESP => {
            if body.len() < 4 {
                return Err("the phone sent a malformed done packet".to_owned());
            }
            Ok(Sahara::DoneResp {
                status: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            })
        }
        other => Err(format!("the phone sent an unexpected Sahara packet ({other:#x})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a CPIO `newc` entry with `name` and `data`.
    fn entry(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(CPIO_MAGIC);
        for field in [
            0,      // ino
            0o100644, // mode
            0,      // uid
            0,      // gid
            1,      // nlink
            0,      // mtime
            data.len(),
            0, // devmajor
            0, // devminor
            0, // rdevmajor
            0, // rdevminor
        ] {
            out.extend_from_slice(format!("{field:08x}").as_bytes());
        }
        let name = format!("{name}\0");
        out.extend_from_slice(format!("{:08x}", name.len()).as_bytes());
        out.extend_from_slice(b"00000000");
        out.extend_from_slice(name.as_bytes());
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    #[test]
    fn a_plain_programmer_is_one_image() {
        let file = std::env::temp_dir().join(format!("lmnflash-fh-{}", std::process::id()));
        std::fs::write(&file, b"programmer bytes").unwrap();

        let images = programmer_images(&file).unwrap();
        assert_eq!(images, vec![Some(b"programmer bytes".to_vec())]);

        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn an_archive_is_split_by_image_id() {
        let mut archive = entry("13:prog_firehose_ddr.elf", b"first image");
        archive.extend(entry("14:second.elf", b"second"));
        archive.extend(entry(CPIO_TRAILER, b""));

        let images = decode_archive(&archive).unwrap();
        assert_eq!(images.len(), 15);
        assert!(images[0].is_none());
        assert_eq!(images[13].as_deref(), Some(&b"first image"[..]));
        assert_eq!(images[14].as_deref(), Some(&b"second"[..]));
    }

    #[test]
    fn an_archive_entry_without_an_id_is_refused() {
        let mut archive = entry("prog_firehose_ddr.elf", b"image");
        archive.extend(entry(CPIO_TRAILER, b""));

        assert!(decode_archive(&archive).is_err());
    }

    #[test]
    fn a_truncated_archive_is_refused() {
        let mut archive = entry("13:prog_firehose_ddr.elf", b"first image");
        archive.extend(entry(CPIO_TRAILER, b""));
        archive.truncate(archive.len() - 20);

        assert!(decode_archive(&archive).is_err());
    }

    /// A 32-bit image request is three `u32` fields.
    #[test]
    fn a_32_bit_image_request_is_read() {
        let mut body = Vec::new();
        body.extend_from_slice(&13_u32.to_le_bytes());
        body.extend_from_slice(&1024_u32.to_le_bytes());
        body.extend_from_slice(&4096_u32.to_le_bytes());

        let Sahara::ReadData { image, offset, len } =
            parse_sahara(SAHARA_READ_DATA, &body).unwrap()
        else {
            panic!("expected an image request");
        };

        assert_eq!((image, offset, len), (13, 1024, 4096));
    }

    /// Newer boot ROMs ask with `0x12`, whose three fields are `u64` — the
    /// packet this page's injection failed on before it was handled.
    #[test]
    fn a_64_bit_image_request_is_read() {
        let mut body = Vec::new();
        body.extend_from_slice(&13_u64.to_le_bytes());
        body.extend_from_slice(&4096_u64.to_le_bytes());
        body.extend_from_slice(&1048576_u64.to_le_bytes());

        let Sahara::ReadData { image, offset, len } =
            parse_sahara(SAHARA_READ_DATA_64, &body).unwrap()
        else {
            panic!("expected an image request");
        };

        assert_eq!((image, offset, len), (13, 4096, 1048576));
    }

    #[test]
    fn an_unknown_sahara_packet_is_refused() {
        assert!(parse_sahara(0x99, &[]).is_err());
    }

    fn gpt_header(entries_lba: u64, count: u32, size: u32) -> Vec<u8> {
        let mut sector = vec![0_u8; 512];
        sector[..8].copy_from_slice(b"EFI PART");
        sector[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        sector[80..84].copy_from_slice(&count.to_le_bytes());
        sector[84..88].copy_from_slice(&size.to_le_bytes());
        sector
    }

    fn gpt_entry(name: &str, start: u64, end: u64) -> Vec<u8> {
        let mut entry = vec![0_u8; 128];
        entry[..16].fill(0x42); // a non-zero partition type GUID
        entry[32..40].copy_from_slice(&start.to_le_bytes());
        entry[40..48].copy_from_slice(&end.to_le_bytes());
        for (index, unit) in name.encode_utf16().take(35).enumerate() {
            entry[56 + index * 2..58 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        entry
    }

    #[test]
    fn the_partition_table_is_read_in_bytes() {
        let header = parse_gpt_header(&gpt_header(2, 3, 128)).unwrap();
        assert_eq!(
            header,
            GptHeader {
                entries_lba: 2,
                entry_count: 3,
                entry_size: 128,
            }
        );

        let mut table = gpt_entry("boot_a", 2048, 4095);
        table.extend(gpt_entry("super", 8192, 16383));
        table.extend(vec![0_u8; 128]);

        let partitions = parse_gpt_entries(&table, 1, 4096, &header).unwrap();
        assert_eq!(partitions.len(), 2);

        let boot = &partitions[0];
        assert_eq!(boot.name, "boot_a");
        assert_eq!(boot.lun, 1);
        assert_eq!(boot.begin_offset(), 2048 * 4096);
        assert_eq!(boot.size_bytes(), 2048 * 4096);

        let super_partition = &partitions[1];
        assert_eq!(super_partition.name, "super");
        assert_eq!(super_partition.size_bytes(), 8192 * 4096);
    }

    #[test]
    fn a_sector_without_the_gpt_signature_is_not_a_table() {
        assert!(parse_gpt_header(&vec![0_u8; 512]).is_err());
        assert!(parse_gpt_header(b"EFI PART").is_err());
    }

    #[test]
    fn a_table_shorter_than_its_header_says_is_refused() {
        let header = GptHeader {
            entries_lba: 2,
            entry_count: 4,
            entry_size: 128,
        };

        assert!(parse_gpt_entries(&vec![0_u8; 256], 0, 512, &header).is_err());
    }

    #[test]
    fn autodetect_tries_the_documented_order() {
        let generic = Path::new("prog_firehose_ddr.elf");

        assert_eq!(
            candidates(StorageType::Autodetect, generic),
            vec![
                StorageType::Emmc,
                StorageType::Ufs,
                StorageType::Nvme,
                StorageType::SpiNor,
                StorageType::Nand,
            ]
        );
        assert_eq!(
            candidates(StorageType::Ufs, generic),
            vec![StorageType::Ufs]
        );
    }

    /// A programmer built for one media puts that media first: asking a UFS
    /// phone for eMMC costs the whole 30 second timeout before it answers.
    #[test]
    fn a_programmer_name_puts_its_storage_first() {
        assert_eq!(
            candidates(
                StorageType::Autodetect,
                Path::new("NB1-0-435J-prog_ufs_firehose_8998_ddr.elf")
            ),
            vec![
                StorageType::Ufs,
                StorageType::Emmc,
                StorageType::Nvme,
                StorageType::SpiNor,
                StorageType::Nand,
            ]
        );

        assert_eq!(
            candidates(
                StorageType::Autodetect,
                Path::new("prog_emmc_firehose_8953_ddr.mbn")
            )
            .first(),
            Some(&StorageType::Emmc)
        );
        // A named media never adds an entry: the order is a permutation of
        // the documented one.
        let preferred = candidates(
            StorageType::Autodetect,
            Path::new("prog_spinor_firehose_9607_ddr.elf"),
        );
        assert_eq!(preferred.len(), 5);
        assert_eq!(preferred.first(), Some(&StorageType::SpiNor));
    }

    #[test]
    fn an_ack_with_a_log_line_is_read() {
        let response = complete_response(
            br#"<data><log value="INFO: &quot;hi&quot;" /><response value="ACK" /></data>"#,
        )
        .unwrap();

        assert!(response.is_ack());
        assert_eq!(response.log_text(), "INFO: \"hi\"");
    }

    #[test]
    fn a_nak_is_not_an_ack() {
        let response = complete_response(
            br#"<data><log value="ERROR: bad storage" /><response value="NAK" /></data>"#,
        )
        .unwrap();

        assert!(!response.is_ack());
        assert!(refusal(&response, "configure").contains("bad storage"));
    }

    #[test]
    fn an_incomplete_response_waits_for_more() {
        assert!(complete_response(br#"<data><log value="INFO" /><respon"#).is_none());
    }

    /// The reply only ends with its `</data>`: raw sector data follows that
    /// tag, so stopping at `<response />` would shift every read by seven
    /// bytes.
    #[test]
    fn a_reply_is_complete_only_after_the_data_tag_closes() {
        assert!(complete_response(br#"<data><response value="ACK" />"#).is_none());
        assert!(complete_response(br#"<data><response value="ACK" /></data>"#).is_some());
    }

    #[test]
    fn a_packet_of_exactly_512_bytes_gets_a_newline() {
        let exact = "a".repeat(512);
        assert_eq!(outgoing(&exact).len(), 513);

        let almost = "a".repeat(511);
        assert_eq!(outgoing(&almost).len(), 511);
    }

    /// Every command is sent with the XML declaration: an 8998 phone NAKed a
    /// `<read>` without it ("Ignoring Unrecognized tag 'data'").
    #[test]
    fn every_packet_carries_the_declaration() {
        assert_eq!(
            packet("<data><read /></data>"),
            format!("{XML_DECLARATION}<data><read /></data>")
        );
        // A command that already spells it out is not given a second one.
        let with_declaration = format!("{XML_DECLARATION}<data />");
        assert_eq!(packet(&with_declaration), with_declaration);
    }

    /// A folder holds one file per partition, and a name that is already
    /// taken is numbered; a single selected partition may go into the exact
    /// file the save dialog returned instead.
    #[test]
    fn a_folder_holds_every_partition_and_a_file_holds_exactly_one() {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-target-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch folder");

        let regions: Vec<Region> = ["boot", "system"]
            .iter()
            .enumerate()
            .map(|(index, name)| Region {
                name: (*name).to_owned(),
                lun: 0,
                start_lba: index as u64 * 4096,
                end_lba: index as u64 * 4096 + 4095,
                sector_size: 4096,
            })
            .collect();

        let folder = DumpTarget::Folder(directory.clone());
        assert_eq!(
            dump_paths(&regions, &folder).expect("both partitions get a file"),
            [directory.join("boot.img"), directory.join("system.img")]
        );

        // A single region writes exactly where the dialog said.
        let named = DumpTarget::File(directory.join("my-backup.bin"));
        assert_eq!(
            dump_paths(&regions[..1], &named).expect("one file, one partition"),
            [directory.join("my-backup.bin")]
        );
        assert_eq!(named.path(), directory.join("my-backup.bin"));

        // Two partitions cannot share one file.
        assert!(dump_paths(&regions, &named).is_err());

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A dump writes one file per partition, and a second dump of the same
    /// partition does not overwrite the first one.
    #[test]
    fn a_dump_file_name_does_not_overwrite_an_earlier_one() {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-dump-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch folder");

        let first = unique_path(&directory, "boot");
        assert_eq!(first.file_name().unwrap(), "boot.img");
        std::fs::write(&first, b"first").expect("the first file");

        let second = unique_path(&directory, "boot");
        assert_eq!(second.file_name().unwrap(), "boot (2).img");
        std::fs::write(&second, b"second").expect("the second file");

        assert_eq!(
            unique_path(&directory, "boot").file_name().unwrap(),
            "boot (3).img"
        );
        // A name a file system cannot carry still lands in the folder.
        let sanitised = unique_path(&directory, "a/b");
        assert_eq!(sanitised.parent(), Some(directory.as_path()));

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A partition is read in as many commands as it takes, and no single one
    /// asks for more than the payload the phone was told about: a `<read>`
    /// beyond it is not answered.
    #[test]
    fn a_dump_read_stays_within_one_payload() {
        for sector_size in [512_u32, 4096] {
            let region = Region {
                name: "system".to_owned(),
                lun: 0,
                start_lba: 0,
                end_lba: 8191,
                sector_size,
            };
            let chunk = (MAX_PAYLOAD_BYTES / region.sector_size as usize).max(1) as u64;

            assert!(chunk > 0);
            assert!(chunk * u64::from(region.sector_size) <= MAX_PAYLOAD_BYTES as u64);
            // The whole partition is covered by whole chunks, and the last one
            // carries the remainder.
            let chunks = region.sectors().div_ceil(chunk);
            assert!((chunks - 1) * chunk < region.sectors());
            assert_eq!(chunks * chunk, region.sectors());
        }
    }

    /// An erase names the whole partition, and the sectors around it stay as
    /// they are: the range comes from the table, not from the storage's size.
    #[test]
    fn an_erase_covers_exactly_the_partition() {
        let region = Region {
            name: "userdata".to_owned(),
            lun: 4,
            start_lba: 0x4000,
            end_lba: 0x4000 + 2047,
            sector_size: 4096,
        };

        let xml = erase_xml(&region);
        assert!(xml.starts_with("<data><erase "));
        assert!(xml.ends_with(" /></data>"));
        assert!(xml.contains("SECTOR_SIZE_IN_BYTES=\"4096\""));
        // 2048 sectors of 4096 bytes, from the first sector of the partition.
        assert!(xml.contains("num_partition_sectors=\"2048\""));
        assert!(xml.contains("start_sector=\"16384\""));
        assert!(xml.contains("physical_partition_number=\"4\""));
    }

    /// The bar under the table follows the whole job, not the partition it is
    /// on: the last byte of the last partition is the only 100 %.
    #[test]
    fn the_overall_progress_counts_every_partition() {
        let regions = [
            Region {
                name: "boot_a".to_owned(),
                lun: 0,
                start_lba: 0,
                end_lba: 127,
                sector_size: 4096,
            },
            Region {
                name: "system".to_owned(),
                lun: 0,
                start_lba: 128,
                end_lba: 128 + 1023,
                sector_size: 4096,
            },
        ];

        let total = total_bytes(&regions);
        assert_eq!(total, (128 + 1024) * 4096);

        // The report of the first partition's last chunk is a quarter of the
        // job, even though that partition is done.
        let first = Progress {
            name: regions[0].name.clone(),
            done: regions[0].bytes(),
            total: regions[0].bytes(),
            overall_done: regions[0].bytes(),
            overall_total: total,
        };
        assert!(first.fraction() > 0.1 && first.fraction() < 0.2);

        let last = Progress {
            name: regions[1].name.clone(),
            done: regions[1].bytes(),
            total: regions[1].bytes(),
            overall_done: total,
            overall_total: total,
        };
        assert_eq!(last.fraction(), 1.0);

        // A job that has not measured anything yet does not jump to full.
        assert_eq!(Progress::default().fraction(), 0.0);
    }

    /// A partition of `sectors` whole sectors, as the table describes it.
    fn partition(name: &str, lun: u8, start_lba: u64, sectors: u64) -> Partition {
        Partition {
            name: name.to_owned(),
            lun,
            start_lba,
            end_lba: start_lba + sectors - 1,
            sector_size: 4096,
        }
    }

    /// The `misc` block is the AOSP `bootloader_message`: the command, a zeroed
    /// status, then the arguments recovery runs. The reference images are
    /// 64/84/85 bytes long, and so are these.
    #[test]
    fn a_boot_loader_message_is_a_command_and_its_arguments() {
        let wipe_data = bootloader_message(WIPE_DATA_COMMANDS);
        assert_eq!(wipe_data.len(), 85);
        assert_eq!(&wipe_data[..13], b"boot-recovery");
        assert!(wipe_data[13..64].iter().all(|byte| *byte == 0));
        assert_eq!(&wipe_data[64..], b"recovery\n--wipe_data\n");

        let fastboot = bootloader_message(FASTBOOT_COMMANDS);
        assert_eq!(fastboot.len(), 84);
        assert_eq!(&fastboot[64..], b"recovery\n--fastboot\n");

        // A plain boot into recovery carries no arguments at all.
        let recovery = bootloader_message("");
        assert_eq!(recovery.len(), 64);
        assert_eq!(&recovery[..13], b"boot-recovery");
        assert!(recovery[13..].iter().all(|byte| *byte == 0));
    }

    /// A normal reboot writes nothing: it only resets the phone.
    #[test]
    fn a_normal_reboot_has_no_steps() {
        let partitions = [partition("misc", 0, 2048, 256)];

        assert!(reboot_steps(RebootAction::Normal, &partitions).unwrap().is_empty());
    }

    /// Enabling OEM unlocking clears FRP and the config partition with a
    /// zero-filled image whose last byte is flagged: the partition has to keep
    /// its size, so it is written rather than erased.
    #[test]
    fn an_oem_unlock_blanks_frp_and_config() {
        let partitions = [
            partition("frp", 0, 4096, 1),
            partition("config", 0, 8192, 2),
            partition("misc", 0, 2048, 256),
        ];

        let steps = reboot_steps(RebootAction::OemUnlock, &partitions).unwrap();
        assert_eq!(steps.len(), 2);

        for (step, name) in steps.iter().zip(["frp", "config"]) {
            match step {
                Step::Blank(region, last) => {
                    assert_eq!(region.name, name);
                    assert_eq!(*last, OEM_UNLOCK_FLAG);
                }
                _ => panic!("{name} is blanked, not erased"),
            }
        }

        // The flag lands in the last byte the partition has, and the rest of
        // the image stays zero.
        let mut image = Image::Blank {
            last: OEM_UNLOCK_FLAG,
        };
        let region = Region::of(&partitions[1]);
        let mut last_packet = vec![0_u8; region.sector_size as usize];
        image.fill(&region, 4096, &mut last_packet).unwrap();
        assert_eq!(last_packet.last(), Some(&OEM_UNLOCK_FLAG));
        assert!(last_packet[..4095].iter().all(|byte| *byte == 0));

        // The first packet is left completely alone.
        let mut first_packet = vec![0_u8; region.sector_size as usize];
        image.fill(&region, 0, &mut first_packet).unwrap();
        assert!(first_packet.iter().all(|byte| *byte == 0));
    }

    /// A phone whose table has neither partition cannot be unlocked.
    #[test]
    fn an_oem_unlock_needs_frp_or_config() {
        let partitions = [partition("misc", 0, 2048, 256)];

        assert!(reboot_steps(RebootAction::OemUnlock, &partitions).is_err());
    }

    /// A factory reset clears the two lock partitions and asks recovery to wipe
    /// the data.
    #[test]
    fn a_factory_reset_erases_the_locks_and_wipes_the_data() {
        let partitions = [
            partition("frp", 0, 4096, 1),
            partition("misc", 0, 2048, 256),
        ];

        let steps = reboot_steps(RebootAction::FactoryReset, &partitions).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(matches!(&steps[0], Step::Erase(region) if region.name == "frp"));

        match &steps[1] {
            Step::Write(region, data) => {
                assert_eq!(region.name, "misc");
                assert_eq!(data, &bootloader_message(WIPE_DATA_COMMANDS));
            }
            _ => panic!("the wipe command goes into misc"),
        }

        // Only the lock partitions the phone has are erased.
        let without_frp = reboot_steps(RebootAction::FactoryReset, &partitions[1..]).unwrap();
        assert_eq!(without_frp.len(), 1);
    }

    /// Recovery and fastbootd are `misc` writes only.
    #[test]
    fn recovery_and_fastbootd_write_a_misc_command() {
        let partitions = [partition("misc", 0, 2048, 256)];

        let recovery = reboot_steps(RebootAction::Recovery, &partitions).unwrap();
        assert_eq!(recovery.len(), 1);
        assert!(
            matches!(&recovery[0], Step::Write(region, data) if region.name == "misc" && data.len() == 64)
        );

        let fastbootd = reboot_steps(RebootAction::Fastbootd, &partitions).unwrap();
        assert_eq!(fastbootd.len(), 1);
        match &fastbootd[0] {
            Step::Write(region, data) => {
                assert_eq!(region.name, "misc");
                // `--fastboot` is what a phone's bootloader hands to recovery
                // to enter fastbootd.
                assert!(String::from_utf8_lossy(data).contains("--fastboot"));
            }
            _ => panic!("the fastboot command goes into misc"),
        }
    }

    /// Without a `misc` partition there is nowhere to put a boot command.
    #[test]
    fn a_boot_command_needs_misc() {
        let partitions = [partition("boot", 0, 2048, 256)];

        for action in [RebootAction::FactoryReset, RebootAction::Recovery, RebootAction::Fastbootd]
        {
            assert!(reboot_steps(action, &partitions).is_err(), "{action:?}");
        }
    }

    /// The progress bar counts the whole job, so the last report of the last
    /// step is the only 100 %, and a written image is counted in whole sectors.
    #[test]
    fn a_reboot_counts_its_whole_job() {
        let partitions = [
            partition("frp", 0, 4096, 1),
            partition("misc", 0, 2048, 256),
        ];

        let steps = reboot_steps(RebootAction::FactoryReset, &partitions).unwrap();
        let total = steps.iter().map(Step::bytes).sum::<u64>();

        // The lock partition plus one sector for the 85-byte command.
        assert_eq!(total, 4096 + 4096);
        assert_eq!(padded(85, 4096), 4096);
    }

    /// `<getstorageinfo>` answers with the size inside its `<response>`
    /// element, and programmers spell the attribute differently.
    #[test]
    fn a_storage_info_reply_names_the_size() {
        for reply in [
            r#"<data><response value="ACK" TOTAL_SECTORS="30535680" /></data>"#,
            r#"<data><response value="ACK" total_sectors="30535680" /></data>"#,
            r#"<data><response value="ACK" TotalSectors="30535680" /></data>"#,
            r#"<data><log value="INFO: x" /><response value="ACK" total_sectors="30535680" /></data>"#,
        ] {
            let response = complete_response(reply.as_bytes()).expect(reply);
            assert_eq!(reported_sectors(&response, 4096), Some(30_535_680), "{reply}");
        }

        // A few programmers answer in kilobytes rather than sectors.
        let response =
            complete_response(br#"<data><response value="ACK" SizeInKbytes="4000" /></data>"#)
                .expect("a reply");
        assert_eq!(reported_sectors(&response, 4096), Some(1000));

        // An answer that names no size at all is not a size of zero: the table
        // still knows how far the LUN reaches.
        let response =
            complete_response(br#"<data><response value="ACK" /></data>"#).expect("a reply");
        assert_eq!(reported_sectors(&response, 4096), None);

        // The reply is still one reply when the sector data of a read follows
        // it, and that data is not part of what was parsed.
        let response =
            complete_response(b"<data><response value=\"ACK\" /></data>\x01\x02")
                .expect("a reply");
        assert!(response.is_ack());
        assert!(response.logs.is_empty());
    }

    /// The storage information is read out of the answer: `label: value` log
    /// lines, the JSON `storage_info` blob expanded, and the attributes of a
    /// `<response>` — every way a programmer reports it.
    #[test]
    fn the_storage_information_is_read_out_of_the_answer() {
        let conversation = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n",
            "<log value=\"Binary build date: Nov 16 2017 @ 03:00:06\"/>\n</data>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n",
            "<log value=\"Calling usb_al_bulk_set_zlp_mode(TRUE) since ZlpAwareHost='1'\"/>\n</data>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n",
            "<log value=\"INFO: Device Block Size in Bytes: 0x1000\"/>\n</data>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n",
            "<log value=\"INFO: UFS Total Active LU: 0x6\"/>\n</data>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n",
            "<log value=\"INFO: {&quot;storage_info&quot;: {&quot;block_size&quot;:4096, \
&quot;num_physical&quot;:6, &quot;prod_name&quot;:&quot;KLUEG8U1FB-B0C1&quot;}}\"/>\n</data>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<data>\n<response value=\"ACK\" />\n</data>",
        );

        let response = complete_response(conversation.as_bytes()).expect("the answer");
        assert!(response.is_ack());

        let facts = storage_facts(conversation, &response);

        // A log line is one fact, its `INFO: ` prefix gone; the prose line
        // without a label is not a fact at all.
        assert!(facts.contains(&("Binary build date".to_owned(), "Nov 16 2017 @ 03:00:06".to_owned())));
        assert!(facts.contains(&("Device Block Size in Bytes".to_owned(), "0x1000".to_owned())));
        assert!(facts.contains(&("UFS Total Active LU".to_owned(), "0x6".to_owned())));
        assert!(
            !facts.iter().any(|(label, _)| label.contains("Calling usb_al_bulk")),
            "{facts:?}"
        );

        // The JSON blob is expanded into the keys it names.
        assert!(facts.contains(&("block_size".to_owned(), "4096".to_owned())));
        assert!(facts.contains(&("num_physical".to_owned(), "6".to_owned())));
        assert!(facts.contains(&("prod_name".to_owned(), "KLUEG8U1FB-B0C1".to_owned())));

        // A programmer that answers with attributes instead reports the same
        // way — and its `value` is not a fact.
        let reply = complete_response(
            br#"<data><response value="ACK" TOTAL_SECTORS="1024" MemoryName="UFS" /></data>"#,
        )
        .expect("the answer");
        let facts = storage_facts(
            r#"<data><response value="ACK" TOTAL_SECTORS="1024" MemoryName="UFS" /></data>"#,
            &reply,
        );

        assert_eq!(
            facts,
            [
                ("TOTAL_SECTORS".to_owned(), "1024".to_owned()),
                ("MemoryName".to_owned(), "UFS".to_owned()),
            ]
        );
    }

    /// A burst of `<data>` elements is one answer, with the `<response>` last.
    #[test]
    fn a_burst_of_data_elements_is_one_answer() {
        let declaration = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n";
        let burst = format!(
            "{declaration}<data>\n<log value=\"INFO: a\"/>\n</data>\
             {declaration}<data>\n<log value=\"INFO: b\"/>\n</data>"
        );

        // Without the `<response>` the answer is not over yet.
        assert!(complete_response(burst.as_bytes()).is_none());

        let all = format!(
            "{burst}{declaration}<data>\n<response value=\"ACK\" />\n</data>"
        );
        let response = complete_response(all.as_bytes()).expect("the answer");
        assert!(response.is_ack());

        // The panel shows one element per line, and nothing else changes.
        assert_eq!(
            split_batches(&all),
            format!(
                "{declaration}<data>\n<log value=\"INFO: a\"/>\n</data>\n\
                 {declaration}<data>\n<log value=\"INFO: b\"/>\n</data>\n\
                 {declaration}<data>\n<response value=\"ACK\" />\n</data>"
            )
        );
    }

    /// A LUN is dumped from its first sector to its last, so the image holds
    /// the partition table as well, and lands in `lun<n>.img`.
    #[test]
    fn a_lun_is_dumped_from_its_first_sector() {
        let lun = Lun {
            number: 4,
            sector_size: 4096,
            sectors: 2048,
        };

        let region = lun.region();
        assert_eq!(region.name, "lun4");
        assert_eq!(region.lun, 4);
        assert_eq!(region.start_lba, 0);
        assert_eq!(region.end_lba, 2047);
        assert_eq!(region.sectors(), 2048);
        assert_eq!(lun.bytes(), 2048 * 4096);
    }

    /// A programmer that does not answer `<getstorageinfo>` still has the
    /// extent of the partitions the table read from the LUN.
    #[test]
    fn the_size_of_a_lun_comes_from_its_last_partition() {
        let partitions = [
            partition("boot", 0, 2048, 128),
            partition("system", 0, 8192, 4096),
            partition("userdata", 4, 0, 131072),
        ];

        assert_eq!(table_extent(&partitions, 0), Some(8192 + 4096));
        assert_eq!(table_extent(&partitions, 4), Some(131072));
        assert_eq!(table_extent(&partitions, 2), None);
    }

    /// A LUN is one file, so several of them cannot be given one name.
    #[test]
    fn one_file_holds_one_lun() {
        let luns = [
            Lun {
                number: 0,
                sector_size: 4096,
                sectors: 1024,
            },
            Lun {
                number: 4,
                sector_size: 4096,
                sectors: 2048,
            },
        ];
        let regions: Vec<Region> = luns.iter().map(Lun::region).collect();

        let named = DumpTarget::File(PathBuf::from("lun.img"));
        assert!(dump_paths(&regions[..1], &named).is_ok());
        assert!(dump_paths(&regions, &named).is_err());
    }

    /// An image is written from the first sector of its LUN, and it may be
    /// shorter than the LUN — but not one byte longer, because the extra
    /// sectors would spill into whatever the phone keeps after it.
    #[test]
    fn an_image_that_does_not_fit_its_lun_is_refused() {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-storage-flash-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch folder");

        let luns = [
            Lun {
                number: 0,
                sector_size: 4096,
                sectors: 2,
            },
            Lun {
                number: 4,
                sector_size: 4096,
                sectors: 2,
            },
        ];

        let write = |name: &str, length: usize| {
            let path = directory.join(name);
            std::fs::write(&path, vec![0x5A_u8; length]).expect("the image");
            LunImage {
                path,
                lun: 4,
            }
        };

        // Exactly the size of the LUN, and shorter: both are written.
        let full = write("lun4.img", 8192);
        assert_eq!(checked_image(&full, &luns).expect("it fits").1, 8192);
        let partial = write("lun4-partial.img", 4096);
        assert_eq!(checked_image(&partial, &luns).expect("it fits").1, 4096);

        // One sector more is refused.
        let oversized = write("lun4-too-big.img", 12_288);
        let error = checked_image(&oversized, &luns).expect_err("it does not fit");
        assert!(error.contains("only has 8192 bytes"), "{error}");

        // An empty file, and a LUN the storage does not have.
        let empty = write("lun4-empty.img", 0);
        assert!(checked_image(&empty, &luns).is_err());
        let elsewhere = LunImage {
            path: directory.join("lun4.img"),
            lun: 7,
        };
        assert!(checked_image(&elsewhere, &luns).is_err());

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A partition image is checked against the range of its partition before
    /// the port is opened, so an image that cannot be written is refused while
    /// nothing has been touched.
    #[test]
    fn a_partition_image_is_checked_against_its_partition() {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-partition-flash-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch folder");

        let region = |name: &str, start_lba: u64, end_lba: u64| Region {
            name: name.to_owned(),
            lun: 0,
            start_lba,
            end_lba,
            sector_size: 4096,
        };

        // Two sectors: 8192 bytes, which is what the partition holds.
        let boot = region("boot_a", 0, 1);
        assert_eq!(boot.bytes(), 8192);

        let write = |name: &str, length: usize, region: Region| {
            let path = directory.join(name);
            std::fs::write(&path, vec![0x5A_u8; length]).expect("the image");
            RegionImage { path, region }
        };

        // Exactly the size of the partition, and shorter: both are written.
        assert_eq!(
            image_length(&write("boot_a.img", 8192, boot.clone()).path, boot.bytes(), "partition boot_a")
                .expect("it fits"),
            8192
        );
        assert_eq!(
            image_length(&write("boot_a-partial.img", 4096, boot.clone()).path, boot.bytes(), "partition boot_a")
                .expect("it fits"),
            4096
        );

        // One sector more is refused, and named.
        let oversized = write("boot_a-too-big.img", 12_288, boot.clone());
        let error = image_length(&oversized.path, boot.bytes(), "partition boot_a")
            .expect_err("it does not fit");
        assert!(error.contains("partition boot_a"), "{error}");
        assert!(error.contains("only has 8192 bytes"), "{error}");

        // An empty file is not an image.
        let empty = write("boot_a-empty.img", 0, boot.clone());
        assert!(image_length(&empty.path, boot.bytes(), "partition boot_a").is_err());

        // The write path repeats the check before it opens the port, so a
        // refusal here never reaches the phone — the port below is not a
        // device at all.
        let mut log = FlashLog::create(directory.join("flash.log"));
        let error = flash_partitions("NOPE", &[oversized], &mut log, &|_| {})
            .expect_err("it does not fit");
        assert!(error.contains("partition boot_a"), "{error}");

        let error = flash_partitions("NOPE", &[], &mut log, &|_| {})
            .expect_err("there is nothing to write");
        assert!(error.contains("no image"), "{error}");

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// The hardware ID the boot ROM answers with carries all four values the
    /// card shows: the model in its low sixteen bits, the OEM above it, and the
    /// SoC above that.
    #[test]
    fn the_identity_is_read_out_of_the_hardware_id() {
        // An MSM8998: the ID `bkerler/edl` prints for one.
        let identity = Identity::of(0x0028_c0e1_0051_a012);

        assert_eq!(identity.msm_id, 0x28c0_e1);
        assert_eq!(identity.oem_id, 0x0051);
        assert_eq!(identity.model_id, 0xa012);
        assert_eq!(identity.hw_id_text(), "0x0028C0E10051A012");
        assert_eq!(identity.msm_id_text(), "0x28C0E1");
        assert_eq!(identity.oem_id_text(), "0x0051");
        assert_eq!(identity.model_id_text(), "0xA012");

        // A phone whose IDs are small still prints four (and six) digits, so
        // the rows line up.
        let identity = Identity::of(0x0000_9600_0000_0001);
        assert_eq!(identity.msm_id, 0x0096_00);
        assert_eq!(identity.msm_id_text(), "0x009600");
        assert_eq!(identity.oem_id_text(), "0x0000");
        assert_eq!(identity.model_id_text(), "0x0001");
    }

    /// The key hash is printed as sixty-four uppercase hex digits — the way the
    /// loaders are matched by — and is simply absent when the boot ROM kept it
    /// back.
    #[test]
    fn the_key_hash_is_printed_as_hex() {
        let mut identity = Identity::of(0x0028_c0e1_0051_a012);
        assert_eq!(identity.pk_hash_text(), None);

        identity.pk_hash = Some(vec![
            0x3c, 0xce, 0xb5, 0x5b, 0x6d, 0x88, 0xa0, 0xbe, 0xa0, 0xe2, 0x4d, 0x96, 0x41, 0x50, 0x0f,
            0xa2, 0x39, 0x73, 0x8b, 0x42, 0x57, 0x50, 0x31, 0x18, 0x8d, 0x3a, 0xaf, 0x92, 0x34,
            0x9b, 0x3b, 0x14,
        ]);

        let text = identity.pk_hash_text().expect("the hash was reported");
        assert_eq!(text.len(), 64);
        assert_eq!(
            text,
            "3CCEB55B6D88A0BEA0E24D9641500FA239738B42575031188D3AAF92349B3B14"
        );
        // The log carries it next to the ids, and says how long it was.
        assert!(identity.log_line().contains("pk hash 3CCEB55B"), "{}", identity.log_line());
        assert!(identity.log_line().contains("(32 bytes)"));
    }

    /// The greeting is read for its protocol version — a Sahara 1 boot ROM has
    /// no command mode, so it is never asked who the phone is.
    #[test]
    fn a_greeting_carries_the_protocol_version() {
        let mut body = Vec::new();
        body.extend_from_slice(&2_u32.to_le_bytes());
        body.extend_from_slice(&1_u32.to_le_bytes());
        body.extend_from_slice(&SAHARA_MAX_PACKET.to_le_bytes());
        body.extend_from_slice(&0_u32.to_le_bytes());
        body.extend_from_slice(&[0_u8; 24]);

        match parse_sahara(SAHARA_HELLO, &body).expect("a greeting") {
            Sahara::Hello { version } => assert_eq!(version, 2),
            other => panic!("a greeting was expected, not {}", other.name()),
        }

        // A greeting too short to hold the version is refused rather than
        // guessed at.
        assert!(parse_sahara(SAHARA_HELLO, &body[..8]).is_err());
    }

    /// The three packets of a boot ROM command: ready, the answer's length,
    /// and the reset response that ends a session nobody uploads to.
    #[test]
    fn the_command_packets_are_read() {
        assert!(matches!(
            parse_sahara(SAHARA_CMD_READY, &[]),
            Ok(Sahara::CommandReady)
        ));
        assert!(matches!(
            parse_sahara(SAHARA_RESET_RESP, &[]),
            Ok(Sahara::Reset)
        ));

        let mut body = Vec::new();
        body.extend_from_slice(&SAHARA_READ_HW_ID.to_le_bytes());
        body.extend_from_slice(&8_u32.to_le_bytes());

        match parse_sahara(SAHARA_EXECUTE_RESP, &body).expect("a command response") {
            Sahara::CommandResponse { command, len } => {
                assert_eq!(command, SAHARA_READ_HW_ID);
                assert_eq!(len, 8);
            }
            other => panic!("a command response was expected, not {}", other.name()),
        }

        assert!(parse_sahara(SAHARA_EXECUTE_RESP, &body[..4]).is_err());
    }

    /// A phone that reports one of Qualcomm's test root keys is unfused: secure
    /// boot is off, so the programmer's key never has to match it.
    #[test]
    fn an_unfused_phone_is_recognised_by_its_test_root_key() {
        // The hashes an MSM8998 in EDL reports with secure boot off.
        let test_roots: [[u8; 32]; 2] = [
            // 7be49b72f9e4337223ccb84d6eccca4e61ce16e3602ac2008cb18b75babe6d09
            [
                0x7b, 0xe4, 0x9b, 0x72, 0xf9, 0xe4, 0x33, 0x72, 0x23, 0xcc, 0xb8, 0x4d, 0x6e, 0xcc,
                0xca, 0x4e, 0x61, 0xce, 0x16, 0xe3, 0x60, 0x2a, 0xc2, 0x00, 0x8c, 0xb1, 0x8b, 0x75,
                0xba, 0xbe, 0x6d, 0x09,
            ],
            // cc3153a80293939b90d02d3bf8b23e0292e452fef662c74998421adad42a380f
            [
                0xcc, 0x31, 0x53, 0xa8, 0x02, 0x93, 0x93, 0x9b, 0x90, 0xd0, 0x2d, 0x3b, 0xf8, 0xb2,
                0x3e, 0x02, 0x92, 0xe4, 0x52, 0xfe, 0xf6, 0x62, 0xc7, 0x49, 0x98, 0x42, 0x1a, 0xda,
                0xd4, 0x2a, 0x38, 0x0f,
            ],
        ];

        for root in test_roots {
            // The programmer is signed against a retail key, and that is fine.
            let preflight = Preflight {
                identity: firehose_identity(Some(root.to_vec())),
                programmer: PathBuf::from("prog_firehose_ddr.elf"),
                programmer_hash: Ok(vec![0x7f, 0xe2, 0x40, 0xdd]),
            };

            assert!(preflight.unfused());
            assert_eq!(preflight.roots_match(), Some(false));
        }

        // A fused phone reports its own key, and there the two must agree.
        let preflight = Preflight {
            identity: firehose_identity(Some(vec![0x7f, 0xe2, 0x40, 0xdd])),
            programmer: PathBuf::from("prog_firehose_ddr.elf"),
            programmer_hash: Ok(vec![0x7f, 0xe2, 0x40, 0xdd]),
        };
        assert!(!preflight.unfused());
        assert_eq!(preflight.roots_match(), Some(true));

        // A boot ROM that kept its hash back is not unfused either — there is
        // simply nothing to compare.
        let preflight = Preflight {
            identity: firehose_identity(None),
            programmer: PathBuf::from("prog_firehose_ddr.elf"),
            programmer_hash: Err("it is neither an ELF nor an MBN".to_owned()),
        };
        assert!(!preflight.unfused());
        assert_eq!(preflight.roots_match(), None);
    }

    /// An identity with the MSM8998's ids and the given key hash.
    fn firehose_identity(pk_hash: Option<Vec<u8>>) -> Identity {
        Identity {
            hw_id: 0x0005_e0e1_0000_0000,
            msm_id: 0x05e0_e1,
            oem_id: 0,
            model_id: 0,
            pk_hash,
        }
    }

    /// The hash a programmer is signed against comes out of its own images: a
    /// `melf` holds several and only some of them carry a chain, so they are
    /// tried until one does.
    #[test]
    fn the_programmers_root_hash_comes_out_of_its_images() {
        // Two images, neither of them a programmer.
        let images = vec![
            None,
            Some(b"not a programmer".to_vec()),
            Some(b"still not one".to_vec()),
        ];
        let error = programmer_root_hash(&images, crate::mbn::Hash::Sha256)
            .expect_err("neither image holds a chain");
        assert!(error.contains("neither an ELF nor an MBN"), "{error}");

        // A programmer that carries no image at all says so.
        let error = programmer_root_hash(&[], crate::mbn::Hash::Sha256)
            .expect_err("there is no image");
        assert!(error.contains("carries no image"), "{error}");
    }
}
