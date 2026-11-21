use super::*;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.02,
        "expected {expected}, got {actual}"
    );
}

fn worked_image() -> ImageMetrics {
    ImageMetrics {
        width: 1280,
        height: 720,
        aspect: 16.0 / 9.0,
        unique_colors: 216_853,
        luma_std: 0.173,
        saturation: 0.378,
        glow: 0.0,
        ocr_words: Some(0),
    }
}

fn worked_audio() -> AudioMetrics {
    AudioMetrics {
        duration: 210.0,
        lufs: -14.6,
        peak: -1.4,
        centroid: 3258.0,
        high_share: 0.039_67,
        build_db: -0.232_3,
        silence: 0.0,
    }
}

#[test]
fn palette_buckets() {
    close(image_palette(100).points, 20.0);
    close(image_palette(256).points, 20.0);
    close(image_palette(2048).points, 10.0);
    close(image_palette(16384).points, 0.0);
    close(image_palette(216_853).points, 0.0);
}

#[test]
fn contrast_and_saturation_curves() {
    close(image_contrast(0.18).points, 10.0);
    close(image_contrast(0.10).points, 0.0);
    close(image_contrast(0.14).points, 5.0);
    close(image_contrast(0.05).points, 0.0);
    close(image_saturation(0.378).points, 5.0);
    close(image_saturation(0.80).points, 5.0 * (1.0 - 0.05 / 0.15));
    close(image_saturation(0.10).points, 0.0);
}

#[test]
fn glow_and_text_penalties() {
    close(image_glow(0.0).points, 5.0);
    close(image_glow(0.02).points, 0.0);
    close(image_glow(0.001).points, 5.0);
    close(image_text(0).points, 10.0);
    close(image_text(1).points, 0.0);
}

#[test]
fn audio_buckets() {
    close(audio_loudness(-14.6).points, 8.0);
    close(audio_loudness(-17.5).points, 0.0);
    close(audio_peak(-1.4).points, 8.0);
    close(audio_peak(-0.5).points, 4.0);
    close(audio_peak(0.5).points, 0.0);
    close(audio_centroid(3258.0).points, 8.0);
    close(audio_centroid(1000.0).points, 0.0);
    close(audio_highs(0.08).points, 8.0);
    close(audio_highs(0.02).points, 2.67);
    close(audio_build(-2.0).points, 8.0);
    close(audio_build(0.0).points, 4.0);
    close(audio_build(2.0).points, 0.0);
    close(audio_silence(0.0).points, 5.0);
    close(audio_silence(1.5).points, 2.5);
    close(audio_silence(2.5).points, 0.0);
    close(audio_duration(210.0, 210.0).points, 5.0);
    close(audio_duration(207.0, 210.0).points, 0.0);
}

#[test]
fn reproduced_worked_example() {
    let image = score_image(&worked_image());
    close(image.points, 29.13);
    close(image.score, 58.25);

    let audio = score_audio(&worked_audio(), 210.0, true);
    close(audio.points, 43.24);
    close(audio.score, 86.48);

    let technical = (image.score + audio.score) / 2.0;
    close(technical, 72.35);
    close(
        WEIGHT_TECHNICAL * technical + (1.0 - WEIGHT_TECHNICAL) * 67.0,
        70.2,
    );
}

#[test]
fn missing_ocr_renormalizes_the_image() {
    let mut metrics = worked_image();
    metrics.unique_colors = 128;
    metrics.luma_std = 0.20;
    metrics.saturation = 0.5;
    metrics.ocr_words = None;
    let score = score_image(&metrics);
    close(score.evaluated, 40.0);
    close(score.score, 100.0);
    assert_eq!(score.skipped, vec!["IMG_TEXT"]);
}

#[test]
fn drafts_skip_the_master_metrics() {
    let score = score_audio(&worked_audio(), 60.0, false);
    close(score.evaluated, 29.0);
    assert!(score.skipped.contains(&"AUD_LOUDNESS"));
    assert!(score.skipped.contains(&"AUD_PEAK"));
    assert!(score.skipped.contains(&"AUD_DURATION"));
}

#[test]
fn parses_ffmpeg_output() {
    let ebur = "  Integrated loudness:\n    I:         -14.6 LUFS\n\n  True peak:\n    Peak:       -1.4 dBFS\n";
    let (lufs, peak) = parse_ebur(ebur);
    close(lufs.unwrap(), -14.6);
    close(peak.unwrap(), -1.4);

    let centroid =
        "lavfi.aspectralstats.1.centroid=3000.0\nlavfi.aspectralstats.1.centroid=3516.0\n";
    close(parse_centroid(centroid).unwrap(), 3258.0);

    let rms = "lavfi.astats.Overall.RMS_level=-20.0\nlavfi.astats.Overall.RMS_level=-17.5\n";
    close(parse_rms(rms).unwrap(), -17.5);

    let segment = "[Parsed_astats_0 @ 0x1] Channel: 1\n[Parsed_astats_0 @ 0x1] RMS level dB: -20.0\n[Parsed_astats_0 @ 0x1] Overall\n[Parsed_astats_0 @ 0x1] RMS level dB: -17.4\n";
    close(parse_last_rms(segment).unwrap(), -17.4);

    let silence = "silence_start: 100.0\nsilence_end: 101.2 | silence_duration: 1.2\nsilence_start: 205.4\nsilence_end: 210 | silence_duration: 4.6\n";
    close(parse_silence(silence, 210.0), 1.2);
}

#[test]
fn counts_only_credible_ocr_words() {
    let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t90.0\tEPIC\n\
5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t90.0\tab\n\
5\t1\t1\t1\t1\t3\t0\t0\t10\t10\t60.0\tSOFT\n\
5\t1\t1\t1\t1\t4\t0\t0\t10\t10\t90.0\t'-H‘\n";
    assert_eq!(count_ocr(tsv), 1);
}
