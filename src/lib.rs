// Copyright (c) 2022 Patrick Amrein <amrein@ubique.ch>
//
// This software is released under the MIT License.
// https://opensource.org/licenses/MIT

use std::io::{BufWriter, Cursor, Write, Read, Seek};

use crc_any::CRC;
use zip::{write::FileOptions, ZipArchive};

static PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
static PNG_END: [u8; 12] = [0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];

static CENTRAL_DIRECTORY_OFFSET_OFFSET: usize = 16;
static LOCAL_HEADER_OFFSET: usize = 42;
static COMMENT_LENGTH_OFFSET: usize = 20;

pub fn unzip_archive<R: Read + Seek>(archive: R) -> Result<Vec<(String, Vec<u8>)>, Box<dyn std::error::Error>> {
    let mut archive = ZipArchive::new(archive)?;
    let mut files = vec![];

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let mut file_bytes = vec![];
        file.read_to_end(&mut file_bytes)?;
        files.push((file.name().to_string(),file_bytes));
    }
    Ok(files)
}

pub fn create_archive_from_files<W: Write>(
    img: &[u8],
    files: &[(String, Vec<u8>)],
    output: W,
)-> Result<(), Box<dyn std::error::Error>> {
    let file = zip_files(files)?;
    create_archive(img, &file, output)
}

pub fn create_archive<W: Write>(
    img: &[u8],
    file: &[u8],
    output: W,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut modifiable_archive = file.to_owned();
    assert!(PNG_SIGNATURE == img[0..8]);

    //here we can insert our junk
    let mut current_pos = 0usize;

    let mut writer = BufWriter::new(output);
    writer.write_all(&img[0..8]).unwrap();
    current_pos += 8;

    let mut length: [u8; 4] = [0, 0, 0, 0];
    length.copy_from_slice(&img[8..12]);
    let header_end = current_pos + (u32::from_be_bytes(length) + 4 + 4 + 4) as usize; //lenght (uint) + crc

    writer.write_all(&img[8..header_end]).unwrap();
    current_pos = header_end;

    //save the earliest occurence of the central directory
    let mut start_of_cd = file.len();
    let mut archive = ZipArchive::new(std::io::Cursor::new(file)).unwrap();
    //hide every zipentry as a png tEXt field
    for i in 0..archive.len() {
        let file = archive.by_index(i).unwrap();
        let file_size = file.compressed_size();
        let end = (file.data_start() + file_size) as usize;
        let start = file.header_start() as usize;

        //length is compressed size + comment + \0
        let mut chunk = Chunk {
            length: ((end - start) + "Comment\0".len()) as u32,
            chunk_type: *b"tEXt",
            chunk_data: &modifiable_archive[start..end],
            crc: 0,
        };
        //calculate crc
        chunk.crc();
        //write chunk
        let bytes_written = chunk.write(&mut writer) as usize;
        if start_of_cd > file.central_header_start() as usize {
            start_of_cd = file.central_header_start() as usize;
        }
        //patch offset in cod
        let new_offset = (current_pos + 4 + 4 + "Comment\0".len()) as u32;
        let new_offset_array = new_offset.to_le_bytes();
        let this_cd = file.central_header_start() as usize;
        modifiable_archive[this_cd + LOCAL_HEADER_OFFSET] = new_offset_array[0];
        modifiable_archive[this_cd + LOCAL_HEADER_OFFSET + 1] = new_offset_array[1];
        modifiable_archive[this_cd + LOCAL_HEADER_OFFSET + 2] = new_offset_array[2];
        modifiable_archive[this_cd + LOCAL_HEADER_OFFSET + 3] = new_offset_array[3];

        current_pos += bytes_written;
    }
    //find IEND
    let iend_loc = find_iend_loc(img);
    //write image data (without iENd)
    writer.write_all(&img[header_end..iend_loc]).unwrap();

    //fix EOCD (offset of central directory)
    current_pos += iend_loc - header_end;
    let new_offset = (current_pos as u32 + 4 + 4 + "Comment\0".len() as u32).to_le_bytes();
    let mut start_of_ecod = modifiable_archive.len() - 4;
    while start_of_ecod > 0 {
        if modifiable_archive[start_of_ecod..(start_of_ecod + 4)] == [0x50, 0x4b, 0x5, 0x6] {
            println!("found start of eocd: {}", start_of_ecod);
            break;
        }
        start_of_ecod -= 1;
    }
    modifiable_archive[start_of_ecod + CENTRAL_DIRECTORY_OFFSET_OFFSET] = new_offset[0];
    modifiable_archive[start_of_ecod + CENTRAL_DIRECTORY_OFFSET_OFFSET + 1] = new_offset[1];
    modifiable_archive[start_of_ecod + CENTRAL_DIRECTORY_OFFSET_OFFSET + 2] = new_offset[2];
    modifiable_archive[start_of_ecod + CENTRAL_DIRECTORY_OFFSET_OFFSET + 3] = new_offset[3];

    //patch comment length (comment length is a short)
    let iend_length = (PNG_END.len() as u16).to_le_bytes(); //crc + iend
    modifiable_archive[start_of_ecod + COMMENT_LENGTH_OFFSET] = iend_length[0];
    modifiable_archive[start_of_ecod + COMMENT_LENGTH_OFFSET + 1] = iend_length[1];

    let eocd_end = start_of_ecod + 22; // strip potential comment from original zip
                                       //write zip COD and EOCD as a PNG tEXt chunk
    let mut comment_chunk = Chunk {
        length: (eocd_end - start_of_cd + b"Comment\0".len()) as u32,
        chunk_type: *b"tEXt",
        chunk_data: &modifiable_archive[start_of_cd..eocd_end],
        crc: 0,
    };
    comment_chunk.crc();
    comment_chunk.write(&mut writer);

    //write iend as comment to zip
    writer.write_all(&PNG_END).unwrap();
    writer.flush().unwrap();
    Ok(())
}

fn zip_files(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let buffer = vec![];
    let mut zip = zip::write::ZipWriter::new(Cursor::new(buffer));
    for (name, f) in files {
        zip.start_file(name, FileOptions::default())?;
        zip.write_all(f)?;
    }
    Ok(zip.finish()?.into_inner())
}

fn find_iend_loc(buf: &[u8]) -> usize {
    let mut pos = 0;
    while pos + 11 < buf.len() {
        if PNG_END == buf[pos..(pos + 12)] {
            return pos;
        }
        pos += 1;
    }
    0
}

struct Chunk<'a> {
    length: u32,
    chunk_type: [u8; 4],
    chunk_data: &'a [u8],
    crc: u32,
}

impl<'a> Chunk<'a> {
    fn crc(&mut self) {
        let mut crc = CRC::crc32();
        crc.digest(&self.chunk_type);
        crc.digest(b"Comment\0");
        crc.digest(self.chunk_data);
        self.crc = crc.get_crc() as u32;
    }
    fn write<T: Write>(&self, writer: &mut BufWriter<T>) -> usize {
        let length_array = self.length.to_be_bytes();
        writer.write_all(&length_array).unwrap();
        writer.write_all(&self.chunk_type).unwrap();
        writer.write_all(b"Comment\0").unwrap();
        writer.write_all(self.chunk_data).unwrap();
        let crc_array = self.crc.to_be_bytes();
        writer.write_all(&crc_array).unwrap();

        4 + 4 + self.length as usize + 4
    }
}
