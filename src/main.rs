use std::io::Read;
use clap::{Arg,App};

fn main() {
    let matches = App::new("ZipInPng")
                        .version("1.0")
                        .author("Patrick Amrein <amrein@ubique.ch>")
                        .about("Embeds ZipArchive into PNG (e.g. for hiding Android Libraries inside the drawable resource)")
                        .arg(Arg::with_name("archive")
                                .short("a")
                                .value_name("ARCHIVE")
                                .takes_value(true)
                                .required(true))
                        .arg(Arg::with_name("png")
                                .short("p")
                                .value_name("IMAGE")
                                .takes_value(true)
                                .required(true))
                        .arg(Arg::with_name("output")
                            .short("o")
                            .takes_value(true))
                        .get_matches();

    let mut archive_file = std::fs::File::open(matches.value_of("archive").unwrap()).unwrap(); 
    let mut file = Vec::new();
    archive_file.read_to_end(&mut file).unwrap();

    let mut img_file = std::fs::File::open(matches.value_of("png").unwrap()).unwrap();
    let mut img = Vec::new();
    img_file.read_to_end(&mut img).unwrap();

    let mut buffer_vec = std::fs::File::create(matches.value_of("output").unwrap_or("output.png")).unwrap();

    zip_in_png::create_archive(&img, &file, &mut buffer_vec).unwrap();
    
}