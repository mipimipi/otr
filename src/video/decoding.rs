use super::cfg;
use anyhow::{anyhow, Context};
use block_modes::block_padding::NoPadding;
use block_modes::{BlockMode, Cbc, Ecb};
use blowfish::BlowfishLE;
use chrono::Datelike;
use md5::{Digest, Md5};
use std::{
    clone::Clone,
    collections::HashMap,
    fs::remove_file,
    fs::File,
    io::prelude::*,
    path::Path,
    str,
    sync::mpsc::{channel, Receiver},
    thread,
};

/// URL of OTR web service to request decoding key
const OTR_URL: &str = "http://onlinetvrecorder.com/quelle_neu1.php";
/// Decoder version to be used in decoding key requests
const DECODER_VERSION: &str = "0.4.1133";
/// Sizes of different parts of the encoded video file
const FILETYPE_LENGTH: usize = 10;
const PREAMBLE_LENGTH: usize = 512;
const HEADER_LENGTH: usize = FILETYPE_LENGTH + PREAMBLE_LENGTH;
/// OTR key values
const PREAMBLE_KEY: &str = "EF3AB29CD19F0CAC5759C7ABD12CC92BA3FE0AFEBF960D63FEBD0F45";
const IK: &str = "aFzW1tL7nP9vXd8yUfB5kLoSyATQ";
/// String to verify that the encoded video file has the right type
const OTRKEY_FILETYPE: &str = "OTRKEYFILE";
/// error indicator in OTR response
const OTR_ERROR_INDICATOR: &str = "MessageToBePrintedInDecoder";
/// Keys of parameters contained in the file header
const PARAM_FILENAME: &str = "FN";
const PARAM_FILESIZE: &str = "SZ";
const PARAM_ENCODED_HASH: &str = "OH";
const PARAM_DECODED_HASH: &str = "FH";
/// Keys of parameters contained in the response to the decoding key request
const PARAM_DECODING_KEY: &str = "HP";
/// Sizes for decoding. MAX_CHUNK_SIZE must be a multiple of BLOCK_SIZE
const BLOCK_SIZE: usize = 8;
const MAX_CHUNK_SIZE: usize = 10 * 1024 * 1024;

/// Map parameter keys to its values (key->value)
type OTRParams = HashMap<String, String>;

/// Part of a video file for concurrent decoding
type Chunk = Vec<u8>;

/// Decode a encoded video file. in_path is the path of the decoded video file.
/// out_path is the path of the cut video file.
pub fn decode(in_path: &Path, out_path: &Path) -> anyhow::Result<()> {
    // MAX_CHUNK_SIZE must be a multiple of BLOCK_SIZE
    if MAX_CHUNK_SIZE % BLOCK_SIZE != 0 {
        return Err(anyhow!(
            "Chunk size [{}] is not a multiple of block size [{}]",
            MAX_CHUNK_SIZE,
            BLOCK_SIZE
        ));
    }

    // retrieve parameters from header of encoded video file
    let mut in_file = File::open(&in_path)?;
    let header_params =
        header_params(&mut in_file).with_context(|| "Could not extract video header from")?;

    // check size of encoded video file
    if (in_file.metadata()?.len() as usize) < file_size_from_params(&header_params) {
        return Err(anyhow!("Video file seems to be corrupt: it is too small"));
    }

    // OTR user and password
    let access_data = cfg::otr_access_data()?;
    // current date
    let now = current_date();
    // get key that is needed to encrypt the payload of the decoding key request
    let cbc_key = cbc_key(&access_data.user, &access_data.password, &now).with_context(|| {
        "Could not determine CBC key for encryption of decoding key request payload"
    })?;
    // get parameters for decoding (particularly the decoding key)
    let decoding_params = decoding_params(
        &cbc_key,
        &decoding_params_request(
            &cbc_key,
            &header_params,
            &access_data.user,
            &access_data.password,
            &now,
        )
        .with_context(|| "Could not assemble request for decoding key")?,
    )
    .with_context(|| "Could not retrieve decoding key")?;

    // decode encoded video file in concurrent threads using the decoding key
    if let Err(err) = decode_in_parallel(
        &mut in_file,
        out_path,
        &header_params,
        decoding_params.get(PARAM_DECODING_KEY).unwrap(),
    ) {
        remove_file(&out_path).unwrap_or_else(|_| {
            panic!(
                "Could not delete file {:?} after error when decoding video",
                out_path
            )
        });
        return Err(err);
    }

    // remove encoded video file
    remove_file(&in_path).with_context(|| {
        format!(
            "Could not remove {:?} after successful decoding",
            in_path.file_name().unwrap().to_str().unwrap()
        )
    })?;

    Ok(())
}

/// Key that is needed to encrypt the payload of the decoding key request
fn cbc_key(user: &str, password: &str, now: &str) -> anyhow::Result<String> {
    let user_hash = format!("{:02x}", Md5::digest(user.as_bytes()));
    let password_hash = format!("{:02x}", Md5::digest(password.as_bytes()));
    let cbc_key: String = user_hash[0..13].to_string()
        + &now[..4]
        + &password_hash[0..11]
        + &now[4..6]
        + &user_hash[21..32]
        + &now[6..]
        + &password_hash[19..32];

    Ok(cbc_key)
}

/// Calculate the sizes of the different chunks for parallel decoding. The
/// result is a vector [MAX_CHUNK_SIZE, ..., MAX_CHUNK_SIZE, CHUNK_SIZE,
/// REMAINDER], whereas CHUNK_SIZE is less than MAX_CHUNK_SIZE but is a multiple
/// of BLOCK_SIZE. REMAINDER is less than BLOCK_SIZE.
fn chunk_sizes(file_size: usize) -> Vec<usize> {
    let (full_chunks, remainder) = (file_size / MAX_CHUNK_SIZE, file_size % MAX_CHUNK_SIZE);
    let mut sizes: Vec<usize> = vec![MAX_CHUNK_SIZE; full_chunks];
    if remainder / BLOCK_SIZE > 0 {
        sizes.push(remainder / BLOCK_SIZE * BLOCK_SIZE);
    }
    if remainder % BLOCK_SIZE > 0 {
        sizes.push(remainder % BLOCK_SIZE);
    }
    sizes
}

/// Current date and returns it as numeric string of format "YYYYMMDD"
fn current_date() -> String {
    let now = chrono::Local::now().date();
    format!("{:04}{:02}{:02}", now.year(), now.month(), now.day())
}

/// Decode one chunk of an encoded video file and return the corresponding
/// decoded chunk. This function is called in a dedicated thread for each
/// chunk.
fn decode_chunk(key: &str, mut chunk: Chunk) -> Chunk {
    // chunks can only be decoded if their size is greater than
    // BLOCK_SIZE. Otherwise, the chunk is returned encoded
    if chunk.capacity() >= BLOCK_SIZE {
        Ecb::<BlowfishLE, NoPadding>::new_from_slices(
            &hex::decode(key).expect("Could not turn decoding key into hex string"),
            &hex::decode("").unwrap(),
        )
        .unwrap_or_else(|_| panic!("Could not create cipher object for decoding of chunk"))
        .decrypt(&mut chunk)
        .unwrap_or_else(|_| panic!("Could not decode chunk"));
    }
    chunk
}

/// Decode a video file (in_file) in concurrent threads using key as decoding
/// key and write the result to out_path
fn decode_in_parallel(
    in_file: &mut File,
    out_path: &Path,
    header_params: &OTRParams,
    key: &str,
) -> anyhow::Result<()> {
    // output file
    let mut out_file = File::create(out_path)
        .with_context(|| format!("Could not create result file {:?}", out_path))?;

    // thread handle to be able to wait until all threads are done
    let mut thread_handles = vec![];

    // create channels and start threads to determine the checksums of the video
    // file before and after decoding, if that is required
    let (enc_hash_sender, enc_hash_receiver) = channel();
    let (dec_hash_sender, dec_hash_receiver) = channel();
    let (enc_hash_handle, dec_hash_handle) = (
        thread::spawn(move || -> [u8; 16] { hashing_queue(enc_hash_receiver) }),
        thread::spawn(move || -> [u8; 16] { hashing_queue(dec_hash_receiver) }),
    );

    // read the chunks sequentially and start and decode each chunk in a
    // separate thread
    for chunk_size in chunk_sizes(file_size_from_params(header_params) - HEADER_LENGTH) {
        // allocate next chunk
        let mut chunk = vec![0u8; chunk_size];

        // read chunk from encoded file and check number of bytes that were read
        if in_file
            .read(&mut chunk[..chunk_size])
            .with_context(|| "Could not read chunk")?
            < chunk_size as usize
        {
            return Err(anyhow!("Chunk is too short"));
        }

        // update hasher to determine the checksum of the encoded file
        enc_hash_sender.send(chunk.clone()).unwrap();

        // decode chunk in new thread. Each thread returns the decoded chunk
        let dec_key = key.to_string();
        thread_handles.push(thread::spawn(move || -> Chunk {
            decode_chunk(&dec_key, chunk)
        }));
    }

    // sender must be dropped explicitely to make hasher thread terminating
    drop(enc_hash_sender);

    // join thread results. I.e., receive chunks and write them to the output
    // file. The chunk sequence is kept by the sequence of thread handles in the
    // thread handles vector
    for handle in thread_handles {
        match handle.join() {
            Ok(chunk) => {
                // update hasher to determine the checksum of the decoded file
                dec_hash_sender.send(chunk.clone()).unwrap();
                // write content to output file
                out_file.write_all(&chunk).with_context(|| {
                    format!("Could not write to decoded video file {:?}", out_path,)
                })?;
            }
            Err(_) => {
                return Err(anyhow!(format!(
                    "Could not create decoded video file {:?}",
                    out_path
                )));
            }
        }
    }

    // sender must be dropped explicitly to make hasher thread terminating
    drop(dec_hash_sender);

    // check MD5 checksums
    if !verify_checksum(
        &enc_hash_handle.join().unwrap(),
        &header_params[PARAM_ENCODED_HASH],
    )
    .context("Could not verify checksum of encoded video file")?
    {
        return Err(anyhow!("MD5 checksum of encoded video file is not correct"));
    }
    if !verify_checksum(
        &dec_hash_handle.join().unwrap(),
        &header_params[PARAM_DECODED_HASH],
    )
    .context("Could not verify checksum of decoded video file")?
    {
        return Err(anyhow!("MD5 checksum of decoded video file is not correct"));
    }

    Ok(())
}

/// Request decoding parameters (incl. decoding key) via OTR web service and
/// return them as hash map: key -> value.
fn decoding_params(cbc_key: &str, request: &str) -> anyhow::Result<OTRParams> {
    // request decoding key from OTR
    let response = reqwest::blocking::Client::builder()
        .user_agent("Windows-OTR-Decoder/".to_string() + DECODER_VERSION)
        .build()
        .with_context(|| "Could not create HTTP client to request decoding key")?
        .get(request)
        .send()
        .with_context(|| "Did not get a response for decoding key request")?
        .text()
        .with_context(|| {
            "Response to decoding key request is corrupted: could not turn into text"
        })?;

    // check for error reported by OTR web service
    if &response[..OTR_ERROR_INDICATOR.len()] == OTR_ERROR_INDICATOR {
        return Err(anyhow!(
            "Error while requesting decoding key: '{}'",
            response[OTR_ERROR_INDICATOR.len()..].to_string()
        ));
    }

    // decode response from base64 format
    let mut response = base64::decode(&response)
        .with_context(|| "Could not decode response to decoding key request from base64")?;

    // check response length
    if response.len() < 2 * BLOCK_SIZE || response.len() % BLOCK_SIZE != 0 {
        return Err(anyhow!(
            "Response to decoding key request is corrupted: must be a multiple of {}",
            BLOCK_SIZE
        ));
    }

    // decode response
    let init_vector = &response[..BLOCK_SIZE];
    let response_decrypted = Cbc::<BlowfishLE, NoPadding>::new_from_slices(
        &hex::decode(cbc_key).with_context(|| "Could not turn CBC key into byte array")?,
        init_vector,
    )
    .with_context(|| "Could not create cipher object for decryption of decoding key response")?
    .decrypt(&mut response[BLOCK_SIZE..])
    .with_context(|| "Could not decrypt decryption key response")?;

    // extract parameters into hash map
    let decoding_params = params_from_str(
        str::from_utf8(response_decrypted)
            .with_context(|| "Reponse to decoding key request is corrupt")?,
        vec![PARAM_DECODING_KEY],
    )
    .with_context(|| "Could not extract decoding parameters")?;

    Ok(decoding_params)
}

/// Assemble the URL for requesting the decoding key via the OTR web service.
fn decoding_params_request(
    cbc_key: &str,
    header: &OTRParams,
    user: &str,
    password: &str,
    now: &str,
) -> anyhow::Result<String> {
    // assemble payload
    let mut payload: String = "&A=".to_string()
        + user
        + "&P="
        + password
        + "&FN="
        + header.get(PARAM_FILENAME).unwrap()
        + "&OH="
        + header.get(PARAM_ENCODED_HASH).unwrap()
        + "&M="
        + &format!("{:02x}", Md5::digest(b"something"))
        + "&OS="
        + &format!("{:02x}", Md5::digest(b"Windows"))
        + "&LN=DE"
        + "&VN="
        + DECODER_VERSION
        + "&IR=TRUE"
        + "&IK="
        + IK
        + "&D=";
    payload += &random_hex_string(512 - BLOCK_SIZE - payload.len());

    // encrypt payload
    let init_vector = random_byte_vector(BLOCK_SIZE);
    let payload_as_bytes = unsafe { payload.as_bytes_mut() };
    let payload_encrypted = Cbc::<BlowfishLE, NoPadding>::new_from_slices(
        &hex::decode(cbc_key).with_context(|| "Could not turn CBC key into byte array")?,
        &init_vector,
    )
    .with_context(|| {
        "Could not create cipher object for encryption of decryption key request payload"
    })?
    .encrypt(payload_as_bytes, 512 - BLOCK_SIZE)
    .with_context(|| "Could not encrypt decryption key request payload")?;

    // assemble value for code parameter
    let mut code = init_vector;
    code.extend_from_slice(payload_encrypted);

    // finally assemble URL
    let request: String =
        OTR_URL.to_string() + "?code=" + &base64::encode(code) + "&AA=" + user + "&ZZ=" + now;

    Ok(request)
}

/// Extract the parameter SZ (= file size) from the header parameter hash map
/// and return it as unsigned integer
fn file_size_from_params(header_params: &OTRParams) -> usize {
    header_params
        .get(PARAM_FILESIZE)
        .unwrap()
        .parse::<usize>()
        .unwrap()
}

/// Calculate the MD5 checksum of video file (in this case the data is received
/// via a queue)
fn hashing_queue(queue: Receiver<Chunk>) -> [u8; 16] {
    let mut hasher = Md5::new();

    for data in queue {
        hasher.update(data);
    }

    // retrieve and return checksum
    let mut checksum = [0u8; 16];
    checksum.clone_from_slice(&hasher.finalize()[..]);
    checksum
}

/// Extract parameters from the beginning of the OTRKEY file and return them in
/// a hash map: key -> value.
fn header_params(in_file: &mut File) -> anyhow::Result<OTRParams> {
    let mut buffer = [0; HEADER_LENGTH];

    // read file header
    if in_file
        .read(&mut buffer)
        .with_context(|| "Could not read file")?
        < HEADER_LENGTH
    {
        return Err(anyhow!("File is too short"));
    }

    // check if file header starts with OTRKEY indicator
    if str::from_utf8(&buffer[0..FILETYPE_LENGTH])? != OTRKEY_FILETYPE {
        return Err(anyhow!("File does not start with '{}'", OTRKEY_FILETYPE));
    }

    // create Blowfish little endian cypher and decrypt rest of file header
    Ecb::<BlowfishLE, NoPadding>::new_from_slices(
        &hex::decode(PREAMBLE_KEY).with_context(|| "Could not decrypt preamble key")?,
        &hex::decode("").unwrap(),
    )
    .with_context(|| "Could not create cipher object for header decryption")?
    .decrypt(&mut buffer[FILETYPE_LENGTH..])
    .with_context(|| "Could not decode file header")?;

    // extract parameters
    let header_params = params_from_str(
        str::from_utf8(&buffer[FILETYPE_LENGTH..])
            .with_context(|| "Decrypted file header is corrupt")?,
        vec![
            PARAM_FILENAME,
            PARAM_FILESIZE,
            PARAM_ENCODED_HASH,
            PARAM_DECODED_HASH,
        ],
    )
    .with_context(|| "Could not extract parameters from file header")?;

    Ok(header_params)
}

/// Extract parameters from a string of the "key1=value1&key2=value2&..." into
/// a hash map: key -> value. must_have is a list of keys that must be present.
/// If any of these keys is missing, an error is returned
fn params_from_str(params_str: &str, must_have: Vec<&str>) -> anyhow::Result<OTRParams> {
    let mut params: OTRParams = HashMap::new();
    for param in params_str.split('&') {
        if param.is_empty() {
            continue;
        }
        // split in key / value and add parameter to map
        let a: Vec<&str> = param.split('=').collect();
        params.insert(a[0].to_string(), a[1].to_string());
    }

    // check if all parameters are there
    for key in must_have {
        if params.get(key).is_none() {
            return Err(anyhow!("Parameter {:?} could not be extracted", key));
        }
    }

    Ok(params)
}

/// Assemble a random byte vector of length len
fn random_byte_vector(len: usize) -> Vec<u8> {
    let mut bytes = Vec::<u8>::new();
    for _ in 0..len {
        bytes.push(rand::random::<u8>());
    }
    bytes
}

/// Assemble a random hexadecimal string of length len
fn random_hex_string(len: usize) -> String {
    random_string::generate(len, "0123456789abcdef")
}

/// Check if checksum fits to hash. The hash must be a 48 character hex string.
fn verify_checksum(checksum: &[u8], hash: &str) -> anyhow::Result<bool> {
    if hash.len() != 48 {
        return Err(anyhow!("MD5 hash must be 48 characters long"));
    }

    // reduce hash length to 32 characters and convert it into bytes array
    let reduced_hash = hex::decode(
        hash.chars()
            .enumerate()
            .filter_map(|(i, c)| if (i + 1) % 3 != 0 { Some(c) } else { None })
            .collect::<String>(),
    )
    .context("Could not turn hash {} into bytes")?;

    Ok(checksum == reduced_hash)
}
