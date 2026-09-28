use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::holdem::{HoldemGameEvaluation, HoldemGameState};
use ducy::games::{GameEquityEvaluation, GameEvaluation};

fn main() {
    let mut game = HoldemGameState::new();
    game.add_player(Deck::parse("As Ac").unwrap()).unwrap();
    game.add_player(Deck::parse("Ks Kd").unwrap()).unwrap();

    game.set_flop(Deck::parse("Kc Qd Js").unwrap()).unwrap();

    let evaluator = HoldemGameEvaluation {};

    let winners = evaluator.evaluate_winners(&game);
    println!("After flop (Kc Qd Js):");
    for w in &winners {
        println!(
            "  Player {} wins with {}",
            w.player_index(),
            w.winning_hand()
        );
    }

    game.set_turn(Card::parse("Tc").unwrap()).unwrap();

    let winners = evaluator.evaluate_winners(&game);
    println!("\nAfter turn (Tc):");
    for w in &winners {
        println!(
            "  Player {} wins with {}",
            w.player_index(),
            w.winning_hand()
        );
    }

    let equity = evaluator.evaluate_equity(&game);
    println!("\nEquity on the turn:");
    for (i, eq) in equity.iter().enumerate() {
        println!("  Player {i}: {eq}");
    }

    game.set_river(Card::parse("Ad").unwrap()).unwrap();

    let winners = evaluator.evaluate_winners(&game);
    println!("\nAfter river (Ad):");
    for w in &winners {
        println!(
            "  Player {} — {} (pot share: {})",
            w.player_index(),
            w.winning_hand(),
            w.pot_amount()
        );
    }
}
