#[derive(Debug)]
pub enum Error {
    IOError(std::io::Error),
    AlsaError { message: String, sys_error: String },
    /// The data is neither a WAV nor an OGG Vorbis file.
    UnsupportedFormat,
    /// Only mono and stereo sources are supported.
    UnsupportedChannels(u16),
    /// Sample format that the decoder does not handle (e.g. 12-bit or ADPCM WAV).
    UnsupportedSampleFormat(String),
    /// The file was recognized but is corrupt or could not be decoded.
    Decode(String),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error {
        Error::IOError(error)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::IOError(e) => write!(f, "audio io error: {e}"),
            Error::AlsaError { message, sys_error } => {
                write!(f, "alsa error: {message} ({sys_error})")
            }
            Error::UnsupportedFormat => write!(f, "unsupported audio format (expected WAV or OGG)"),
            Error::UnsupportedChannels(n) => {
                write!(f, "unsupported channel count: {n} (expected 1 or 2)")
            }
            Error::UnsupportedSampleFormat(s) => write!(f, "unsupported sample format: {s}"),
            Error::Decode(s) => write!(f, "audio decode error: {s}"),
        }
    }
}

impl std::error::Error for Error {}
