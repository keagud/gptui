#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    SqliteError(#[from] rusqlite::Error),

    #[error("Error attempting to handle database value: {0}")]
    DbRetrievalError(Box<dyn std::error::Error + Sync + Send>),

    #[error("HTTP request returned status {status}")]
    HttpError {
        base_err: reqwest::Error,
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("Connection error: {base_err}")]
    ConnectionError { base_err: reqwest::Error },

    #[error("Error when processing json: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Channel error: {0}")]
    ChannelError(String),

    #[error(transparent)]
    SendError(#[from] crossbeam_channel::SendError<String>),

    #[error(transparent)]
    ReceiveError(ChannelRecvError),

    #[error(transparent)]
    IOError(#[from] std::io::Error),

    #[error(transparent)]
    ClipboardError(#[from] arboard::Error),

    #[error(transparent)]
    CliError(#[from] clap::Error),

    #[error("Error when communicating with daemon: {0}")]
    CommunicationError(Box<dyn std::error::Error + Sync + Send>),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Sync + Send>),
}

impl From<crossbeam_channel::TryRecvError> for Error {
    fn from(value: crossbeam_channel::TryRecvError) -> Self {
        Self::ReceiveError(ChannelRecvError::TryRecvError(value))
    }
}

impl From<crossbeam_channel::RecvError> for Error {
    fn from(value: crossbeam_channel::RecvError) -> Self {
        Self::ReceiveError(ChannelRecvError::RecvError(value))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelRecvError {
    #[error(transparent)]
    TryRecvError(crossbeam_channel::TryRecvError),

    #[error(transparent)]
    RecvError(crossbeam_channel::RecvError),
}

#[allow(unused)]
macro_rules! other_err {

    ($x:expr) => {

        {
            crate::Error::Other($x.into())

        }


};
    ($($x:expr),+) => {
        {

            crate::Error::Other(anyhow::format_err!($($x),+).into())

        }

    };
}

#[allow(unused)]
pub(crate) use other_err;

pub type Result<T> = std::result::Result<T, crate::Error>;

impl From<anyhow::Error> for Error {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.into())
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        if let Some(status) = value.status() {
            let message = value.to_string();
            Self::HttpError {
                base_err: value,
                status,
                message,
            }
        } else {
            Self::ConnectionError { base_err: value }
        }
    }
}
