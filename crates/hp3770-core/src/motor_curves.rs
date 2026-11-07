// SPDX-License-Identifier: GPL-2.0-or-later
//! Motor acceleration and deceleration curves of the HP Scanjet 3770.
//!
//! Each number is the length of one motor step in clock ticks: the larger,
//! the slower. The curves come from the configuration embedded in the HP
//! driver (`hpgt3770.dll`), which holds twelve `[MotorSetting]` sections,
//! each with its `[ACC]` and `[DEC]` assignments and its `[AccCurveN]` /
//! `[DecCurveN]` steps. Only the numbers are copied, with their source, as
//! `hp3900` does for its models in `hp3900_config.c`.
//!
//! The configuration of the 3770 refers to the settings by number
//! (`MotorCurve=` in `[MotorMove1]` and in each scan mode). In the file as
//! extracted, setting 10 is the first section, 11 the second and 12 the
//! third; settings 1-9 follow in order, so setting 9 is the last section.
//! The captures show this mapping: the step tables the HP driver sends match
//! setting 10 at 300 dpi (`[ScanColorMode3] MotorCurve=10`, capture 04),
//! setting 11 at 200 dpi (`[ScanColorMode4] MotorCurve=11`, capture 01) and
//! setting 9 when the carriage goes home (`[MotorMove1] MotorCurve=9`,
//! captures 05, 06 and 11), in length, pointers and every step.

use crate::motor::MotorSetting;

/// Motor setting 9, used to bring the carriage home (`[MotorMove1]`).
/// Last `[MotorSetting]` section of the extracted file, line 11941:
/// `[ACC] NormalScan=1 ParkHome=2 Smearing=3`,
/// `[DEC] NormalScan=1 ParkHome=2 Smearing=3 ScanBufferFull=4`.
pub const SETTING_9: MotorSetting = MotorSetting {
    acc_curves: &[ACC_1, ACC_2, ACC_3],
    dec_curves: &[DEC_1, DEC_2, DEC_3, DEC_4],
    acc_normal: 1,
    acc_park: 2,
    acc_smearing: 3,
    dec_normal: 1,
    dec_park: 2,
    dec_smearing: 3,
    dec_buffer_full: 4,
};

/// `[AccCurve1]` (line 11956), 210 steps.
const ACC_1: &[u16] = &[
    3136, 3136, 3136, 3136, 3136, 3136, 2036, 1815, 1664, 998, 998, 915, 850, 797, 752, 715, 682,
    654, 628, 606, 586, 568, 551, 536, 522, 509, 497, 485, 475, 465, 456, 447, 439, 431, 424, 417,
    410, 404, 398, 392, 387, 381, 376, 371, 367, 362, 358, 354, 350, 346, 342, 338, 335, 331, 328,
    325, 322, 319, 316, 313, 310, 308, 305, 302, 300, 298, 295, 293, 291, 288, 286, 284, 282, 280,
    278, 276, 274, 272, 271, 269, 267, 265, 264, 262, 261, 259, 257, 256, 254, 253, 252, 250, 249,
    247, 246, 245, 244, 242, 241, 240, 239, 237, 236, 235, 234, 233, 232, 231, 230, 228, 227, 226,
    225, 224, 223, 222, 221, 221, 220, 219, 218, 217, 216, 215, 214, 213, 213, 212, 211, 210, 209,
    209, 208, 207, 206, 206, 205, 204, 203, 203, 202, 201, 200, 200, 199, 198, 198, 197, 196, 196,
    195, 195, 194, 193, 193, 192, 192, 191, 190, 190, 189, 189, 188, 187, 187, 186, 186, 185, 185,
    184, 184, 183, 183, 182, 182, 181, 181, 180, 180, 179, 179, 178, 178, 177, 177, 176, 176, 175,
    175, 175, 174, 174, 173, 173, 172, 172, 172, 171, 171, 170, 170, 170, 169, 169, 168, 168, 168,
    167, 167, 166,
];

/// `[AccCurve2]` (line 12170), 212 steps.
const ACC_2: &[u16] = &[
    3136, 3136, 3136, 3136, 3136, 3136, 2036, 1815, 1664, 998, 998, 915, 850, 797, 752, 715, 682,
    654, 628, 606, 586, 568, 551, 536, 522, 509, 497, 485, 475, 465, 456, 447, 439, 431, 424, 417,
    410, 404, 398, 392, 387, 381, 376, 371, 367, 362, 358, 354, 350, 346, 342, 338, 335, 331, 328,
    325, 322, 319, 316, 313, 310, 308, 305, 302, 300, 298, 295, 293, 291, 288, 286, 284, 282, 280,
    278, 276, 274, 272, 271, 269, 267, 265, 264, 262, 261, 259, 257, 256, 254, 253, 252, 250, 249,
    247, 246, 245, 244, 242, 241, 240, 239, 237, 236, 235, 234, 233, 232, 231, 230, 228, 227, 226,
    225, 224, 223, 222, 221, 221, 220, 219, 218, 217, 216, 215, 214, 213, 213, 212, 211, 210, 209,
    209, 208, 207, 206, 206, 205, 204, 203, 203, 202, 201, 200, 200, 199, 198, 198, 197, 196, 196,
    195, 195, 194, 193, 193, 192, 192, 191, 190, 190, 189, 189, 188, 187, 187, 186, 186, 185, 185,
    184, 184, 183, 183, 182, 182, 181, 181, 180, 180, 179, 179, 178, 178, 177, 177, 176, 176, 175,
    175, 175, 174, 174, 173, 173, 172, 172, 172, 171, 171, 170, 170, 170, 169, 169, 168, 168, 168,
    167, 167, 166, 166, 166,
];

/// `[AccCurve3]` (line 12386), 32 steps.
const ACC_3: &[u16] = &[
    3136, 3136, 3136, 3136, 3136, 3136, 2036, 1815, 1664, 998, 998, 915, 850, 797, 752, 715, 682,
    654, 628, 606, 586, 568, 551, 536, 522, 509, 497, 485, 475, 465, 456, 455,
];

/// `[DecCurve1]` (line 12422), 32 steps.
const DEC_1: &[u16] = &[
    455, 456, 465, 475, 485, 497, 509, 522, 536, 551, 568, 586, 606, 628, 654, 682, 715, 752, 797,
    850, 915, 998, 998, 1664, 1815, 2036, 3136, 3136, 3136, 3136, 3136, 3136,
];

/// `[DecCurve2]` (line 12458): `Count=1 32` and 32 steps listed. The HP
/// driver sends one step: the last table of every capture is a single 455.
const DEC_2: &[u16] = &[455];

/// `[DecCurve3]` (line 12494), 32 steps.
const DEC_3: &[u16] = &[
    455, 456, 465, 475, 485, 497, 509, 522, 536, 551, 568, 586, 606, 628, 654, 682, 715, 752, 797,
    850, 915, 998, 998, 1664, 1815, 2036, 3136, 3136, 3136, 3136, 3136, 3136,
];

/// `[DecCurve4]` (line 12530), 126 steps.
const DEC_4: &[u16] = &[
    110, 110, 111, 111, 112, 112, 113, 113, 114, 114, 115, 115, 116, 116, 117, 117, 118, 118, 119,
    119, 120, 120, 121, 121, 122, 122, 123, 123, 124, 124, 125, 125, 126, 126, 127, 127, 128, 128,
    129, 129, 130, 131, 132, 133, 133, 134, 135, 136, 137, 138, 139, 140, 141, 143, 144, 145, 146,
    147, 149, 150, 151, 153, 154, 156, 157, 159, 161, 162, 164, 166, 168, 170, 172, 174, 176, 179,
    181, 184, 186, 189, 192, 195, 198, 202, 206, 209, 213, 218, 222, 227, 233, 239, 245, 252, 259,
    267, 276, 282, 286, 291, 298, 305, 310, 319, 325, 331, 342, 354, 362, 376, 387, 404, 417, 431,
    456, 475, 509, 551, 586, 654, 715, 850, 998, 1664, 3136, 3136,
];
