//! Compressed sections inside 1.0 save files.
//!
//! `ZPackage.GetCompressed` / `ZPackage.Decompress` wrap a package in gzip
//! (`Utils.Compress`, `GZipStream` at its fastest level). Since gzip output
//! depends on the implementation, a section read from a file the game wrote
//! and written back here decodes to the same bytes without being the same
//! bytes, so nothing built on this can promise byte-identical round trips.

use binrw::io::{Cursor, Read, Seek, Write};
use binrw::{BinRead, BinResult, BinWrite, Endian};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

/// An `i32` byte count followed by a gzip stream whose content is a `T`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gzipped<T>(pub T);

impl<T> BinRead for Gzipped<T>
where
    T: for<'a> BinRead<Args<'a> = ()>,
{
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, _: Endian, _: ()) -> BinResult<Self> {
        let pos = reader.stream_position()?;
        let len = i32::read_le(reader)?;
        let len = usize::try_from(len).map_err(|_| binrw::Error::AssertFail {
            pos,
            message: format!("negative compressed section length {len}"),
        })?;
        let mut compressed = vec![0u8; len];
        reader.read_exact(&mut compressed)?;
        let mut plain = Vec::new();
        GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut plain)
            .map_err(|e| binrw::Error::Custom {
                pos,
                err: Box::new(e),
            })?;
        let mut cursor = Cursor::new(plain.as_slice());
        let value = T::read_le(&mut cursor)?;
        let consumed = cursor.position() as usize;
        if consumed != plain.len() {
            return Err(binrw::Error::AssertFail {
                pos,
                message: format!(
                    "compressed section has {} unparsed trailing bytes",
                    plain.len() - consumed
                ),
            });
        }
        Ok(Gzipped(value))
    }
}

impl<T> BinWrite for Gzipped<T>
where
    T: for<'a> BinWrite<Args<'a> = ()>,
{
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, _: Endian, _: ()) -> BinResult<()> {
        let mut plain = Cursor::new(Vec::new());
        self.0.write_le(&mut plain)?;
        let pos = writer.stream_position()?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let compressed = encoder
            .write_all(&plain.into_inner())
            .and_then(|()| encoder.finish())
            .map_err(|e| binrw::Error::Custom {
                pos,
                err: Box::new(e),
            })?;
        (compressed.len() as i32).write_le(writer)?;
        writer.write_all(&compressed)?;
        Ok(())
    }
}
