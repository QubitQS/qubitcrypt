use ml_kem::B32;
use openssl::pkey::Id;
use sha2::Digest;

use crate::kdf::common::kdf_trait::Kdf;
use crate::kdf::sha3::Sha3Kdf;
use crate::kdfs::KdfType;
use crate::kem::common::kem_info::KemInfo;
use crate::kem::common::kem_trait::Kem;
use crate::kem::common::kem_type::KemType;
use crate::utils::openssl_utils;
use crate::QubitCryptError;

use crate::kem::ec_kem::EcKemManager;
use crate::kem::ml_kem::MlKemManager;

type Result<T> = std::result::Result<T, QubitCryptError>;

/// A KEM manager for the Xwing method
pub struct XWingKemManager {
    kem_info: KemInfo,
    ml_kem: MlKemManager,
    ec_kem: EcKemManager,
    shake: Sha3Kdf,
}

impl XWingKemManager {
    #[allow(clippy::type_complexity)]
    fn expand_decapsulation_key(&self, sk: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> {
        let expanded = self.shake.derive(sk, &[], 96, None)?;
        let d: B32 = expanded[0..32]
            .try_into()
            .map_err(|_| QubitCryptError::InvalidPrivateKey)?;
        let z: B32 = expanded[32..64]
            .try_into()
            .map_err(|_| QubitCryptError::InvalidPrivateKey)?;
        let (pk_m, sk_m) = self.ml_kem.key_gen_deterministic(&d, &z)?;
        let sk_x = expanded[64..96].to_vec();
        let pk_x = openssl_utils::get_pk_from_sk_pkey_based(&sk_x, Id::X25519)
            .map_err(|_| QubitCryptError::InvalidPrivateKey)?;

        Ok((sk_m, sk_x, pk_m, pk_x))
    }

    fn combiner(&self, ss_m: &[u8], ss_x: &[u8], ct_x: &[u8], pk_x: &[u8]) -> Result<Vec<u8>> {
        let xwing_label = b"\\.//^\\";
        let mut info = xwing_label.to_vec();
        info.extend_from_slice(ss_m);
        info.extend_from_slice(ss_x);
        info.extend_from_slice(ct_x);
        info.extend_from_slice(pk_x);

        // Get the SHA3-256 hash of the info
        let mut sha3 = sha3::Sha3_256::default();
        sha3.update(&info);
        let result = sha3.finalize_reset();
        Ok(result.to_vec())
    }
}

impl Kem for XWingKemManager {
    fn new(kem_type: KemType) -> Result<Self>
    where
        Self: Sized,
    {
        let kem_info = KemInfo::new(kem_type);
        let ml_kem = MlKemManager::new(KemType::MlKem768)?;
        let ec_kem = EcKemManager::new(KemType::X25519)?;
        let shake = Sha3Kdf::new(KdfType::Shake128)?;
        Ok(XWingKemManager {
            kem_info,
            ml_kem,
            ec_kem,
            shake,
        })
    }

    fn get_kem_info(&self) -> KemInfo {
        self.kem_info.clone()
    }

    fn key_gen(&mut self) -> Result<(Vec<u8>, Vec<u8>)> {
        // Use OpenSSL to generate 32 bytes of random data
        let mut sk = vec![0u8; 32];
        openssl::rand::rand_bytes(&mut sk).map_err(|_| QubitCryptError::KeyPairGenerationFailed)?;

        // Expand the secret key
        let (_, _, pk_m, pk_x) = self.expand_decapsulation_key(&sk)?;

        // Concatentate the public keys
        let pk = [pk_m.as_slice(), pk_x.as_slice()].concat();

        // returns the 32 byte secret decapsulation key sk and
        // the 1216 byte encapsulation key pk

        Ok((pk, sk))
    }

    fn key_gen_with_rng(
        &mut self,
        rng: &mut impl rand_core::CryptoRngCore,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        // Use the provided RNG to generate 32 bytes of random data
        let mut sk = vec![0u8; 32];
        rng.fill_bytes(&mut sk);

        // Expand the secret key
        let (_, _, pk_m, pk_x) = self.expand_decapsulation_key(&sk)?;

        // Concatentate the public keys
        let pk = [pk_m.as_slice(), pk_x.as_slice()].concat();

        // returns the 32 byte secret decapsulation key sk and
        // the 1216 byte encapsulation key pk

        Ok((pk, sk))
    }

    fn encap(&mut self, pk: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        if pk.len() != 1216 {
            return Err(QubitCryptError::InvalidPublicKey);
        }
        let pk_m = &pk[0..1184];
        let pk_x = &pk[1184..1216];

        let (ss_x, ct_x) = self.ec_kem.encap(pk_x)?;
        let (ss_m, ct_m) = self.ml_kem.encap(pk_m)?;

        let ss = self.combiner(&ss_m, &ss_x, &ct_x, pk_x)?;
        let ct = [ct_m.as_slice(), ct_x.as_slice()].concat();

        Ok((ss, ct))
    }

    fn decap(&self, sk: &[u8], ct: &[u8]) -> Result<Vec<u8>> {
        let (sk_m, sk_x, _pk_m, pk_x) = self.expand_decapsulation_key(sk)?;
        if ct.len() != 1120 {
            return Err(QubitCryptError::InvalidCiphertext);
        }

        let ct_m = &ct[0..1088];
        let ct_x = &ct[1088..1120];

        let ss_m = self.ml_kem.decap(&sk_m, ct_m)?;
        let ss_x = self.ec_kem.decap(&sk_x, ct_x)?;

        let ss = self.combiner(&ss_m, &ss_x, ct_x, &pk_x)?;

        Ok(ss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kem::common::kem_trait::Kem;
    use crate::kem::common::kem_type::KemType;
    use crate::kem::common::macros::test_kem;

    #[test]
    fn test_xwing() {
        let kem = XWingKemManager::new(KemType::XWing);
        test_kem!(kem);
    }

    #[test]
    fn test_xwing_vectors() {
        // Test vectors from the XWing KEM specification
        // https://datatracker.ietf.org/doc/html/draft-connolly-cfrg-xwing-kem-04
        let sk = hex::decode("7f9c2ba4e88f827d616045507605853ed73b8093f6efbc88eb1a6eacfa66ef26")
            .unwrap();
        let _pk = hex::decode("ec367a5c586f3817a98698b5fc4082907a8873909a7e79ce5d18c84d425362c7956aeb610b9b68949107335ba676142aadd93ed27211c57c2d319c805a01204f5c2948158759f06327825379ac8428113e59c215a582a50d0c89505390ecf868e9270cf96c4fa0f94793b5998b38a1e3c9039bf4147695c08413a7e190b5d9a8c22a4759bda51fd11064a21c4dadc3bc28510697f3a442205214a6cd3f54a9ba45a309437d372654e2c6226e8640810128597abe9042932be6af1eb71a6ef156a9baa4c0c05764a8314fc1565d1825a5eb3604f278bc175b0666af85a13d97c650a571564eca080a36727bf76460c81a842895e87c9d4fc9c57fc6b149692eed526fb632cd476232a9f3035b4c96d6a14f8cf92e2735a766c7a168e6034369b6c17750afcc483af5654b82439f6b9a136cb4f47986dab4c427327675061d7b130572e2071f22339a997cf1e1618133ac8b8acd1d7177943c0d1971c84fc48cce7c4c00b95a9f77414c4c07fb3b0c6d51144d36cc8be4ae9b236f89accdd4336bcff11f4fc997ef13c01bb45d4001b1949749ebf14e469788ebdbbeced68ba149ca81aab111d0756f1074b7e60031da437709027c4676edc35318a74b1308a8f2b6aef905668bb031a6403ab7a328ba74b9231866e287424b42acd1d69b6eab657f2340f433717e581a048ac9be5196fedc36ec212de48149bbec9e07ccc8b1f50293e78e469079a3d3588ae146c1859ced376dc13040c4535f253cb40a61b8be95b8b6606d2f607c1035a23566ade289391829ae61cacd36d247a3a864bab43b23198481f10f9a5b25b64cb6314baaa0282c59792fe987687b06cb23b397302962cacb9f7327301310c7e66b9f5aab93b0f9ba9b5633a1db72fa637c4f6611ca9117788bb335b80dd0c989af6b0d8fc9b5c3707a1d848b220a3002b612c294a004c4b52ad1b4b57619d960a659646622a73de9a55de1191dcf8253b50bb2d6e0bed3ab12c4bb81b2826afec87dabccb56b74bdd4c844005097ac94cafea715a57b6e20b49e49869bfdc8015e37a0b3f942f9467b7c749f76c951623340660bbd88c16dfbf5176ca855689bbf7287391935b71eda6ef8bab6a2ea6e3095a1f2719d10b205130982942c1bbad0bb6c1901879587ac3a290ff20043010e181337eb2a20eda44b24e07f12255bbe78279adc51de276d2e602b72dc1ed7489240ab2c4e672b527082e363b0b5f51ffbbb79d724435484ca0c7874aff654d61a254eb7ae420b4d0a9958a48144e013972cda7f8adcc7c36206725221a79426e7c798e99cb645198c506194c3da36415501ea6bccb377921f0172cf9634232b211d626074020cdec29c4d59248c405688f15d6bc556f72bb01d11ae0b2167d33bb2389a2d6dec911a3513fc680d21a265c3f3b190e983d5bab1ae471802024edfd96a2cd51176261107c29f5050ab52ca7210db8668bb80064744cb4236e3ac6df26477c8d80ac9a60ca8796f95c5acd960b2f541027c2378ac15708070acfa528a8473248458cb3cf23108949369009b523a945fc70cf3c3add61c4fbbdba91d74c954682182d30071e71648f1b266ea343ab97547c9a3462969ca911a67667e1cb88467942eea1ae5d06ac215e64de876fda67c22f74ffe26ff8b56cf606ff799d4a89bb6cee3f79506960abcda4e65d8197e0c992244dae91c21068915647f844f49").unwrap();
        let ct = hex::decode("b45085dc0c2abecd811415924ade853ae88c8dcf8007e6d79bae036648290472989d6f2187bc6d39d0f739d315fc03cd8a373ad8927b0db7d419385c9b867b351815a95e7f0f915e7356eacce50d328a572565c538b282dc539e4d4b106ba5add0656efb8bd670a32e89fb642eae8235fdc181b2a3ae21d5f3374ce6955484c4fa9dd0a8e454f73e840fa5085070d10789e3cc1f6b4274fad17c041c23a8c512e3be23962de5028f427273f5a53dcf43425e9183d304abf22b306fb6add4c89a7b54fa93d50393882bad23e06c58c03cbb765a9d1324be9fe7b399b7a0f7486b8b03fe186dc5e9ee9738f48e7ef3127a6db992097263dbc51fb227dfab0aae2758d8cfd8573c227e19d245503518ee7f533976236075d50f95b5bd101c670714209f264c01e31b80295fea54f42e1c62856042bafbe72e1ef8abe12f58b02e4eb6378bc0e13339395b6faf95e2738c509975bc1806d1cbad3e586cfa2ba09b2bde20dfb0aaba2cdb583ae33c812109a1095adc697befcbd0be0aafee1e41979be026747c918646d38874320aaf404f28cda6d6d7a7a5386f487983a69064b8bc1fc0a2998a55bb442cfa9b61581263b33f5ae25c4a1efdd890c3fae4481995eaabf1d4a27addc239b99bb8aefec73a9f9c15819026d35d48e11de426f7f113e8fe843db011934c8052300cca9fc870f390648ab47ff543629949c5459fae763871e949a4d2f61caf9f6afcfbc00e5b71f85c791ae04d4db90ed09811382a8a2a9707f76cbeaa371eb64d2a8d82e1f65b42e0928e5afa288062ca0b28317c9b36b27f14161d84d71db377efc6f0f2d7b57594e8fc432c2dbcbc4f55fc3563894a5be4ad40a2aa34ca48db0df5b6d8ae51777bf7c6925a40e651629351e86480594f438ee3a34daa7a2581e0f573489e71b23bf76dcf8fd3d9c29ca6bcc699753d54b876adb0c0514ae887e1029ef195fc3cddb51d03cb518f8dad5044e2299f601b961fa38da47d1e940b58e864cf5dbe85a21dafc40b2355144307d09bd2bf8b1c762e7bd5e27308d903e165ecc6176b74564329bf37e1ce9257d113897c0099aaa17937735dd13931c5742f5cceaec475c1886bfef42252a7ad66f4d4b925faec8e1a9ce0623a895e9c00c57781e66404311720bb94ff0c019081f9b846d72451179308f17d4c7ac324a5bbbb914411840364b9b65f6e189c60ef842c155df1f96b84f03521803d3cb7016629b4c8159fb0ad3ce1da5e49ceba56f6881be8432200c86e291a4cd3b5ea9001e99b418b9d44a3fa0cedb6acf3feef30df4307480967e765530d6183add3a198d796a4535abbd8be92d8c2f9ec4217fd459326f0f090764b57207d4cb108af34abf120c182011e66393edf2f446f606acb5b0ad5afb4ea5866e4d4158280885bd0ad4deced058ced8035afc85d1e03c00b7c23b4e74abe8ba12b86a027064bf88443aadb38c82bc621b6880d3e88f6c3bcb03a015d1cc306f7d575ee778cd1b52902be555b4e02b74cfd310bd83ab4c81f97fc12e56f17576740ce2a32fc5145030145cfb97e63e0e41d354274a079d3e6fb2e15").unwrap();
        let ss = hex::decode("555a071a8b7520ae95f8e635de8a5f87dbddcbef900576aad29ecdda5459c15a")
            .unwrap();

        let kem = XWingKemManager::new(KemType::XWing).unwrap();
        let result = kem.decap(&sk, &ct).unwrap();
        assert_eq!(result, ss);

        let sk = hex::decode("badfd6dfaac359a5efbb7bcc4b59d538df9a04302e10c8bc1cbf1a0b3a5120ea")
            .unwrap();
        let _pk = hex::decode("8b92541ff9b60f49baf1282229853e3249427656bda5440c6f258df3db3382c2b5674590fd713397954b785694807938653603666b103ea8709b002a420a7bee64161015631c6c32e7c6a53d525eb44940733667447439d830280e7ba75f501958a09ce8e3b7d0038a7bbc22e7025700686662f128ce0771d92a8c8af4cf2a414c11b19c695b234bb447471c67aa5a94bad89e192563bb33662fda91d24845f0e3b08eac79d03635fe81610e80bb8bcb92fc05cc5a12c7d7a5340a81c031d55a6b81b7469ca14c72cf2634b686cac7060a759700d01885119a557a3a23487a5432dce5932f3a3b0c85972b5caf711c5f21066ff7011743d16a22da06b7e8ba22d390746034617b2f47c0c392f926a9665ba89244f5b30a1e9388c2a4749c9b70c7ba3a2c18c32481373a35acf8157a561613ddb693040b8f319876a7c165ab53aee1b4071d782f84e6cb415565ed4c0c00222e1a5c41bd8485eb40a4053183f2753f1c5243a536624f940029307d5d984934d05ce623066bc29ea6b93a3da99c927218f366ae98e168b30c536dec49f108c9f353073e347137604597053f20a03abd71280d235022a03c9d0a43ee29340845ca51f848056ca98506b91f61b26b9582a0556748a56f36d115a0365da0390b22db09d8c1029d3494ce811878e68e56295c38712c16f098f89b5a5953b568e7618af069a63c0f39ca70f73a0f2cbb150a64884e2c3d491b5013689741715b02227176d767715060da0439f39698283428e4b939f9fbbde627ce1be73c19fb54d082b7399383b5ac44e1205dfae12e6672601a4cc89a3a80677447e4c53b127b05c528a0e95cb512eb9dc8799ec00c38e427bf5ff94775877b87109c69b0bd367223bfe6a65f8b889eeb7a2ad80ea6001654d350cd665420d34f80d99fef72cb124089331585d6c5bd6d057f8e319352db71878196e6587d2d018f1496043bac9fd9106d53c30510b6abefd80d3d6319f1cbb5937b3d808706f888c2c07840d3a0c53565603909073660b83fc318c775b34909001054b82f9a0cc8155a3b002a12a414bad1a69dc2aa1aa489af7965a16989148467539352dd1794356473a3aca6668b5cd006ab972236d7484536ec1029d845fb386c9e4a539e593bfb225f558459e28274fccb0a05ac47528b459171c76764a47ed8b461f3124c174aa0c51778299a0a299882a63e88510d9adb479c30b47950974b9556cef689bf5006354c4fe11684a9d057f09c213ea63c32228ce5657f336a88bda34fff435d5a682a44312da5bc8d8a6b7bfc1a60fab9ce7a268c00dc040f04d0b651adf4e31ad6b36981b7a1ccd666a80b0f89928b81b686ce3c6b18e4a921c0a4f4d26bf0cb568db77908424b47d8031bb860c19a1f5eb131e838aa5e628bf29b2dfadc0c6e6c8dcb8277b783b61b69a303007f3a49018f3592c505618f3c3aeaf775995c6097671ea0cab7633c612a24a144dc81abc28fcc62b2b69a1e93d8b6e93ca81a2b9cd49bc1b5373af699aaefac7ba66c110198ad91555b9a46ac19322870322e16ec8cfee384c241bc21e28d5a48af52d68f37a8bb0669b7b1630889083f5e6bcd99007183014304553ed8d602a3b12df2b26e846c4f1a04eed59a7f1855399a47636186d461695ee4ff1f2558e47c05824395bb484b686c33a4dbc1d068a4423693afc0154b88c241340fa9bf6623011dea34").unwrap();
        let ct = hex::decode("04f8b82b60ce1b47119f9b199c02a0be5b709394e6171c8b272509371681701c4d99bc4bee7f16772bb596271f8c80ed215cf777ce1507785a7506b3cc8a9ee516f0f3a90f4af6cff30115a944dc2856b6ec40ff9678dcfd81563ea5db1a4d269c8f1a7fdfbb9f6b44bdaee6a7126a143097e758463ad827298990b34170f655591e640a3b9db75610918534b3741283e2a0c50c716a8263e8dda9db348e7bab6d1a249f27fb899d05a3bc1cbb2ab0f7b1a378b1803bb83af411cf434fba3cd28bcb19ebedb675111359a3aa8575d3868e945fbada53bc9e555a0988820a39728b35930762b28509efc7bfd8d046cca32a9896c8071d56c8a508a26bdcd58e64696e15813ab66dabbfedb0d6bfa2078cf98131142765c6f9f54a501da1518a9a33206ee0fc7145c3cbca30e8b92354b21cecf6e5aced0ce9909a4204f3b007119dcdb303f1ff5a3352b78dff6f42a4ffcabe220938665316044fe533f01000e54f8e03f912b7b5263d329cfed70dfcf5059fb795426be342b6a849940469579d0f464aa9bba2dbe170c3532f0d8ece7bd71b993ca07c09ad010f4966525bdc43a1e2db0bd1b05aad5f70ded58df621ee73503f15037e42a130ab2962fcde2b1f10a518ed3c4cd37d1f1ba5e6f21231f4ff2f3925b5536186c57ae4efcbaef6ff59c261f1b61cc759c44863efe438b2ca3ad150adfcd5b7cbdf15ea8d2d7213d362a6364e558c94454425f5705844b2e0fdaa4965aa082c6b2e0e8256ce530752f79697af3fd5fd2c42a5bd64633106be2b26f370253c92e8212e9dfcad3ca10ccb69e0c8ebe6607c98bf0c75329949601133e554bf86cfecbc6c2093062655b6be07311ce7aaf48d24ea4448c45838af1fdf564c8af1a5948ff26d3e61ab3edf6c89570c78ceda2042eeae695143b592313d2c010b0911c796a6c90bba167a3705c5f7ccaab2ff21ed8ca281949b5b16f8a09be2ed7736903a85c20cf7f6bd48797a20f5225fe90c9ee10a81f8acfd6a81747dbe63753b5e594507e14e0a228ea99d4ae51d8caac0dc2f769989c967ebc80dc44c9e2f140ec29eab1d372f2370b0f66779337578b91354140c2998f19028c1e5f7634e94bfd470f3fbd9ab3f853093262bd403fbb18c1d28d8ac1b90f7b3e99936868bc0ce192ac338cf32baf87aeb9b71de977502c77230d4881ad30998c24b767790f16cc56c41e1781debb5c6f9d37b27120a3f225bfdf64ed65807fe988ea2c3001f711070a8354ba92468b115445af1bd8a9942287f4ffccaacaa715c34bc24901687f257808e48d7a2b6c49f4dbe46fc628819f09f89ac553da58e06e03553b30cdd3b715d4367b768da472fc2e734a47ab79ea9e30ad475a4de23321f8c6c9551e57629180cdde9923a90add52165195c7e670bd5989558307eaa0e6513d0225ba1fa213b319c378649bba7a1ddc146b5528d9230f0bc13c2922565d01ce23b57c6058dcc0a3153bdd6c2f0fa5394fb0a506cded4ab50a587a63cec6db3393790e0de35c4c309d88a41c91bdf6e0e03200693c9651e469aee6f91c98bea4127ae66312f4ae3ea155b67").unwrap();
        let ss = hex::decode("d99c3cd6cde624a73b2f80d9be695c5ab804a42fcca392bc2fd8504b81b2bf6e")
            .unwrap();

        let result = kem.decap(&sk, &ct).unwrap();
        assert_eq!(result, ss);

        let sk = hex::decode("ef58538b8d23f87732ea63b02b4fa0f4873360e2841928cd60dd4cee8cc0d4c9")
            .unwrap();
        let _pk = hex::decode("9512abd457939c591d51687e8bc2ac7beace759bccb208b113729902d98543479384ac627ca21424304ad9281119f3460c975b6c3612812c5bd1f2422e3773a5c21c6d3862d6c3777ee31822a5718402543cf06476171cbf5895332a444e06c35a4c217d57768574a89dc0a58c8160efd847d0861579c278648b92a863b65c688eb41c1adf453d9494cf14f60c339c0b3e51a937d16cc1491314e013d8cacde2bcbf05d5994658579c834be29b5f6a1a601e24552a46822cf99838a57154916415dc54ea3363d751ced5b01160677417c81be3ba67c9b190bf29c85567b3051b17d6d75b97b883b043c442da3994910296a44f17a187bb51367fc88756baab782a45550b3e1c733bbac4b8a668b0ab586f64450d6a05219047785c1956a086207ac0a636d4bb398abf1649adf0e2069bb7ad063273575ac7947cc25beb768312464874c4f2091a3c434d93867943b0a3ebc22962aa2b6d5b284e0133f3309fde0c848c5436dd4772b1bcb81793affee19ab76c598d76b851282ac13ccfc7d4c6a56b2c336c70bc87221dc163c5518ebb3a5807e918033aacc2a3ad5e555131bb6d61c6b1446882a19c36bf142b7e7375a6893799b835a1a755383184c0027dbbfc193e546c9389cc53216280ab9c39323d11d468ca255e0af805712932656bba50f92528d435cca903d6981a1c2b4c9cb30cbdb4bd5f1a5ed84855ad076a18477f202bc699a412c4471a98d9b5974935a5151ef3167c7f8baccfd9432504bc7c6081491887d872c25fa7a6dd80cb3946832e828cf3406a1a0522ab3b63bdfb102d13ac2f193e7e7c82f6ec3804e2220c83bd83eb1bd0d65e175c81e7e176b3f6116d6140ac409ba4a30d1ef61e7fa0bb88ac209588c75bfa01aab803f55a8f77dc6eee6c0bf34c2c99e9aa846a2a590a757182ca8fd7536c0920e39a4fa3f763ea307f67172bacd6c2bc8937856b0eb56959bcfac111f964620081b7d380d3000f27f88a26da2db8f12493f641b09c9f65a563ebb913e267b1132ba46734cb111cb230d39f1bface22666b42529a01041bccd132bb0852995ccd273c384748a78848c284f5447eecaf9fa6c244977b676a1d2c26a8dc65c7974c89482189f4d545fd57348c6b670853cc6b383773925eaf336103aa5da5609b4da3c91ce54d2832974f3c445b5424a6182ca792b76de08e96518abaa94afc51646169301ce49722c13b5a1bc2ecac1a2408589531cc371a6064268db2bb8be783790efb00e38037cbc294e8820c4a9694785b3a8653320dfa296741b9658c6c7c41c91b0875d205191238b28ca860a48231f0733d87254ad5572d11e2955645589f0b19651a943ee20c1c7387b706083e093711eb099f4c32e4f26f4ee9ad3e74aeb4624c42c030cd66ad4087cb0cfa5f33323a286379c2090df78b1b04f1cc0cf931f6d6a49bcc6540e4c59373c6ebe64ca38886d75451e2192bfd20a7aeb528dcf1437665294099c09b2560d66ca900d33dea091d1c9a346c66731941cf57522b6fe33f0a27085e23b72e689990f14622111506dc14e2427db9285fbc82a90f9a23510746fd6caaf8418b59d622e16ca1edd1c1a452b48b0ab62a7369bd1722d3413a868b1c6ccdf2e301ec1f272ba68a1b5e02d743257be8ba94e9eb0f3ec27ce36306ca7b11ab356886212ddf193c9915762936f787a3bf665fc381c954c3bb0297da7836").unwrap();
        let ct = hex::decode("5dae4188bb92ba700d830d8dc6aaf6ef699dcd48ac93c3f5b1e4b0ffc22cf21dfcb51be22532844f061bf3b9393c96dad49050bc9244584675c6919232893288f7218ba84d77f21b84cdccc514c147b54372b60bd8ee81c3dc96f0bee2eeadddca79e3d483f7d9e71fed01f5d6399ad9d7ede107b859b0f4031ea1376691ce6bfe36d1a211ceee3361035117e12ec81e3ab6362cfb7b87a0262279a2efd632a704dcea32fa1036a49e631d6d253ef49be6b46b7844b963bc05e7ca76b203594a2fa5f2812659a5e7aa74531128343f086c80cc69f3cd90ef4fecf1336f796b0c1c3d15cc54b4b7ee653f4c3b1e3cf97a4c1b223da34e90d6905974ab4e544ce2af8fdbefe8d5a3e8479ee07afb4ef546dc86d22e62081a27c14613ec1d284f850e96c4b4a8283f5b307d4f43776f86d9e5a7dbcf3073d468ed6f8cb82f8a0b13947703fd15c6911c31379e1d1ccc11560044554a4403cad162a4d86ef22be6a794a240af90e8367affbe4f84c8a4cd76934c1aea726efbf156108de461a276ac34baabc8bdb33085ce5dd9b6d89ef66f06c0590ae7a0cb8fd79a16ee4bdf48aa9788093e6e2f6fa63e837aa80072c2fbe5503a22b9cfff87d67b4f7508885966e8b5332299fe95f6e023d813e8e44f79d271eb725687d2dd5e06c0fd34cf5ce9108709960a4d23ef8e8f3d3ba65f5ff8bb57d07c6398ba46f7a81eba115fa2132066c96fbd1f7bce0749d1f961516c2d86b6f98f3b6f5d4a0dbb530cd7c59f4cad1554dd434bf44be5559614aa1b72ef41b179d667827c458b4196675dd538e288bdaebfbe57f13d65ed6cfcf38b5017dcd38c60a8dec0685944c86535b6a3c8a206bacea9d26b4d34cf1d28b4e6aead08f5ba668c0e8bd032ace996f112f7ae5d176ac0bccc2ff8b25e15ff7b7350e045d41b499179c1a754184fffaff17fa97025437bf590eb142e472d17a331e0a54045928a73ed641ac57f1b88b1392198949e625d0b2c5e7e9d721d02d609eca58a3fd830e6daa97ed59cd5466e356203fbe33e5947bf8ece898ecd2740ded4bb9e5633554fbe4a72b5564207099e13937bac1e7d7abfa843e95f6293282db0cd1dcd5cf1f3333d982343bcd6de04e9f6ded75b7f395b97790c564fc6be98ef5e18d9ab26c193d6ea2afb4a5771255745b727b8734bc06111b4a1d589d56a176cc5f5ea84d36f0402a9830aa5d7029bc65e3042dc85c66bc8a5c848690f8a847ad4ee837fd8eb01d5b5e29c5781afbd321351073f73b2fc7517f87e4a72e0206212ae393a7ce82a8144d4ab3b7eadd2efed21a6913ea298a0813d1b26c61a0f93623fa587b12bbacd163a76365023b2fc2b09aeea255dedca9d94b17ac0577435cc627d53992fafe73b956443b2ac1b8f9572c8b3f648f4e1b6427f90b45709f6c18d87e9e8f4d85e747abd90d7889ba92a27620f2ef6e674ec2f289da1b4e1c9ade9623582ff69aaad4f7131382fc15b0f2132ab0df74d769421d1d1677619cff13f1c2097bc6b098a7bbca0111658f0fa64de6b6e1c3c8e03db5971a445992227c825590688d203523f527161137334").unwrap();
        let ss = hex::decode("3c111a7476821d18c4c05192298a881c46de82a25035e1fbc1ec399cd5a29924")
            .unwrap();

        let result = kem.decap(&sk, &ct).unwrap();
        assert_eq!(result, ss);
    }
}
