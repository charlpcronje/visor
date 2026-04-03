use anyhow::{Context, Result};
use windows::core::PCSTR;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, ERROR_PIPE_CONNECTED,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileA, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeA, DisconnectNamedPipe,
    PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::models::PIPE_NAME;

const BUFFER_SIZE: u32 = 65536;

/// Server-side: create a named pipe instance and wait for a client.
pub struct PipeServer {
    handle: HANDLE,
}

unsafe impl Send for PipeServer {}

impl PipeServer {
    pub fn create() -> Result<Self> {
        let pipe_name = format!("{PIPE_NAME}\0");
        unsafe {
            let handle = CreateNamedPipeA(
                PCSTR(pipe_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,           // max instances
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,           // default timeout
                None,
            )?;

            if handle == INVALID_HANDLE_VALUE {
                anyhow::bail!("CreateNamedPipeA returned INVALID_HANDLE_VALUE");
            }

            Ok(Self { handle })
        }
    }

    /// Wait for a client to connect. Returns true if a client connected.
    pub fn wait_for_client(&self) -> Result<bool> {
        unsafe {
            match ConnectNamedPipe(self.handle, None) {
                Ok(()) => Ok(true),
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(true),
                Err(e) => Err(e.into()),
            }
        }
    }

    /// Read a length-prefixed message from the pipe.
    pub fn read_message(&self) -> Result<Vec<u8>> {
        read_length_prefixed(self.handle)
    }

    /// Write a length-prefixed message to the pipe.
    pub fn write_message(&self, data: &[u8]) -> Result<()> {
        write_length_prefixed(self.handle, data)?;
        // Flush to ensure client receives data before we disconnect
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(self.handle);
        }
        Ok(())
    }

    /// Disconnect the current client so the pipe can accept a new one.
    pub fn disconnect(&self) {
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(self.handle);
            let _ = DisconnectNamedPipe(self.handle);
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Client-side: connect to an existing named pipe.
pub struct PipeClient {
    handle: HANDLE,
}

unsafe impl Send for PipeClient {}

impl PipeClient {
    pub fn connect() -> Result<Self> {
        let pipe_name = format!("{PIPE_NAME}\0");
        unsafe {
            let handle = CreateFileA(
                PCSTR(pipe_name.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .context("Failed to connect to visor daemon pipe")?;

            Ok(Self { handle })
        }
    }

    pub fn send_request(&self, data: &[u8]) -> Result<()> {
        write_length_prefixed(self.handle, data)
    }

    pub fn read_response(&self) -> Result<Vec<u8>> {
        read_length_prefixed(self.handle)
    }
}

impl Drop for PipeClient {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

// Helper: write a 4-byte length prefix then the payload.
fn write_length_prefixed(handle: HANDLE, data: &[u8]) -> Result<()> {
    let len = data.len() as u32;
    let len_bytes = len.to_le_bytes();
    raw_write(handle, &len_bytes)?;
    raw_write(handle, data)?;
    Ok(())
}

// Helper: read a 4-byte length prefix then the payload.
fn read_length_prefixed(handle: HANDLE) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    raw_read(handle, &mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 10 * 1024 * 1024 {
        anyhow::bail!("IPC message too large: {len} bytes");
    }
    let mut buf = vec![0u8; len];
    raw_read(handle, &mut buf)?;
    Ok(buf)
}

fn raw_write(handle: HANDLE, data: &[u8]) -> Result<()> {
    use windows::Win32::Storage::FileSystem::WriteFile;
    let mut written = 0u32;
    unsafe {
        WriteFile(handle, Some(data), Some(&mut written), None)
            .context("WriteFile to pipe failed")?;
    }
    Ok(())
}

fn raw_read(handle: HANDLE, buf: &mut [u8]) -> Result<()> {
    use windows::Win32::Storage::FileSystem::ReadFile;
    let mut total = 0usize;
    while total < buf.len() {
        let mut read = 0u32;
        unsafe {
            ReadFile(
                handle,
                Some(&mut buf[total..]),
                Some(&mut read),
                None,
            )
            .context("ReadFile from pipe failed")?;
        }
        if read == 0 {
            anyhow::bail!("Pipe closed unexpectedly");
        }
        total += read as usize;
    }
    Ok(())
}
